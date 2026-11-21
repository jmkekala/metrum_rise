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
| Model contract and pilot pair | planned; pilot pair built in the models repository | `TOOLS-41` |
| `[vehicle]` assets in packs, vehicle drafts | planned | `TOOLS-41` |
| Bundled `vehicles` pack, removal of the placeholder cars | planned | `TOOLS-42` |
| Vehicle renderer: catalogue, shader, tiers | planned | `RENDER-65` |
| Brake lights, indicators, steered wheels | planned | `RENDER-66` |
| Low and high beam, later lamps, glass features | planned | `RENDER-67` |
| Lane length by body class | planned | `TRAFFIC-38` |
| Waiting position at junctions | planned | `TRAFFIC-39` |
| Placeholder models, night lamps | as built | `RENDER-26` |

## Model contract (`TOOLS-41`)

**Planned; the pilot pair is built to it in the models repository ([Pilot pair](#pilot-pair)),
nothing on the game side is.** This is the authoritative copy; the models repository's own
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
[the renderer's id decode](#planned-renderer-render-65).

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

`export/<name>.model.json`, schema `metrum-model-facts`, **version 2**, with `"kind":
"vehicle"`. Building sidecars stay version 1 (see
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
- Seats are not in the sidecar. Seating is a value of the pack asset on the game side.
- The lists and LOD0 agree both ways: no listed group is missing from the mesh, and no id in
  the mesh is missing from the lists.

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

Still to do on the game side, as the planned sections below describe: the sidecar reader and the
pack asset ([pack assets](#planned-pack-assets-drafts-and-the-bundled-pack-tools-41-tools-42)),
tier switching, lamp shading and the paint multiply, skipped (white) for a `paint: false`
variant ([renderer](#planned-renderer-render-65)), and wheel steer and spin on the twin wheel
parts as on single ones ([steered wheels](#planned-brake-lights-indicators-and-steered-wheels-render-66),
[wheel spin](#planned-beams-later-lamps-and-glass-render-67)), and the look at the `car` box
with the estate beside a bay ([open points](#open-points-of-the-contract)).

### Open points of the contract

- The `car` box (4.50 × 1.80 m) is looked at once more with the pilot estate beside a parking
  bay, before the fleet is ordered. This is the only open point.

Closed 2026-11-21: the id attribute (`COLOR_0` as normalised unsigned bytes, see
[Vertex ids](#vertex-ids)) and the glass texel mask for check 8.

## Sizes

### Sizes the simulation uses today

**As built.** The lane model and the parking model disagree: lanes were fitted to the placeholder
cars, parking to real bodies.

| What | Value | Where |
| --- | --- | --- |
| Length a vehicle takes in a lane: every car, every truck and every walker | 2.6 m (`CAR_LENGTH`) | `rust/src/config.rs` |
| Standstill gap | 0.1 m (`IDM_S_MIN`) | `rust/src/config.rs` |
| Time headway | 0.5 s (`IDM_T_HEAD`) | `rust/src/config.rs` |
| Car body for lot proofs | 1.8 × 4.5 m, turn square 4.85 m, clear height 2.0 m, smallest bay 1.8 × 4.5 m | `CLASS_TABLE` in `rust/src/simulation/parking/validation.rs` |
| Truck body for lot proofs | 2.6 × 8.0 m, turn square 8.45 m, bay 3.0 × 8.0 m, gate 4.0 m, clear height 4.0 m | the same table |
| Kerbside bay | 6.0 m long in a 2.5 m parking lane | `KERB_BAY_LENGTH_M` in `rust/src/simulation/parking/kerb.rs`, `PARKING_LANE_WIDTH_M` in `rust/src/simulation/network/graph/cross_section.rs` |
| Bays of the bundled lots | 2.5 × 5.0 m | `godot/bootstrap/mods/parking/assets/` |
| Driving lane | 3.5 m (`LANE_WIDTH`) | `rust/src/config.rs` |
| Walker | 1.80 m tall | `godot/assets/models/characters/Quaternius/VAT/` |

`CAR_LENGTH` is read by: the gap to the vehicle ahead, the attach-slot test, the roundabout
entry gap and the follow gap (`rust/src/simulation/economy/agents/tick/traffic/occupancy.rs`),
overlap correction (`tick/lane_buckets/occupancy.rs`), the lane-change gap
(`tick/traffic/lane_change.rs`), evacuation spacing (`agents/building_refs.rs`) and the lane-entry
audits (`rust/src/nodes/sim/core/populated_city/frame_budget/lane_entry_audit.rs` and its
`waits.rs`). Car following itself is specified in [`traffic.md`](traffic.md#car-following).

Consequences today:

- A queue packs one car per 2.7 m, while the bays those cars park in are 5 to 6 m long.
- At 50 km/h the spacing is 2.6 + 0.1 + 6.95 = 9.65 m, about 5,180 vehicles an hour per lane; a
  jammed lane holds 370 cars per km.
- The freight "truck" is drawn as a 3.25 m van and takes 2.6 m in a lane, while its docks, gates
  and turn squares are proven for a 2.6 × 8.0 m body.

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

### Planned: lane length by body class (`TRAFFIC-38`)

**Planned; lands with the real-size models.**

- Length by body class from the one class table the lot proofs use: car 4.5 m, van 6.0 m, truck
  8.0 m. A lane gap becomes the distance between centres minus half of each length.
- Walkers get a spacing of their own; today they borrow the car's.
- Every test listed above reads the class length. The lookups stay O(log cars in a lane) with
  no allocation.
- Standstill gap, headway and acceleration are not retuned in this work.

Expected from the model's own formula (not measured):

| | Today (2.6 m) | Cars at 4.5 m | Trucks at 8.0 m |
| --- | --- | --- | --- |
| Jam spacing | 2.7 m | 4.6 m (41 % fewer cars per km) | 8.1 m |
| Spacing at 50 km/h | 9.65 m | 11.55 m (16 % less flow per lane) | 15.05 m |
| Roundabout entry gap | 5.48 m | 7.38 m | |

- Simulation outcomes change: queues are longer and spill back sooner, and pull-outs from
  driveways and kerb bays wait for larger gaps.
- The fixture cities hold cars 2.7 m apart and are regenerated; the envelope baseline and the
  economy contract get new reference rows.
- The save format stays. A loaded jam is denser than the new minimum, so a test shows that a
  loaded dense city settles without a car leaving its lane.
- Parking needs no change: bays, proofs and kerb bays already fit the class boxes.

### Planned: waiting position at junctions (`TRAFFIC-39`)

**Planned check.** A vehicle lane ends on its junction mouth
([lane ends](traffic.md#junction-connectors)), and a waiting car stands with its centre, not its
nose, at the lane's end. Half its body is past the mouth: 1.3 m today, 2.25 m for a 4.5 m car,
4 m for a truck. This was read from the code, not seen in a still. With the pilot car, one still
at a junction with a crossing decides it: if the body stands on the crossing or in the
junction, queue heads hold half a body length before the lane end, compared against a control.

## As built today

### Models and loading

**As built.**

- Five third-party models are engine files, not pack assets:
  `godot/assets/models/vehicles/civilian/` (sedan, sports sedan, SUV, luxury SUV) and
  `godot/assets/models/vehicles/freight/` (delivery van). They are imported as `keep` and read
  raw.
- `godot/scripts/renderers/vehicle_resources.gd` holds the model table and a hand-written GLB
  reader that reads position, normal and one UV set (`RENDER-59`).
- Models are drawn at their native size: the loader applies no scale and moves the lowest point
  to the ground.
- Each model is turned 180 degrees at load, and the runtime points the car's `-Z` along travel
  (`rust/src/nodes/sim/core/snapshot/agents.rs`).
- Body and wheels are merged into one surface with one material, a shared colour atlas
  (`RENDER-46`).
- Colour is five copies of each mesh with shifted UVs into the atlas; the van has one colour.
- The simulation knows five vehicle types: 0 to 3 are the four cars, 4 is freight
  (`VEHICLE_*` in `rust/src/simulation/economy/agents/mod.rs`); type 4 is also read by freight
  logic. A resident's car keeps a saved model and colour
  (`rust/src/simulation/parking/data.rs`), chosen by a hash when the car is created.

### Drawing

**As built.**

- Moving cars: one `MultiMeshInstance3D` per bucket in `godot/scripts/renderers/agents.gd`. The
  bucket key is `vehicle_type * 10 + colour`, 25 buckets. A car is 12 floats, one transform; no
  instance colour and no custom data. The buffer format is listed in
  [`reference.md`](reference.md).
- The simulation thread poses the cars in view into a snapshot
  (`rust/src/nodes/sim/core/snapshot/agents.rs`); a background job smooths the poses and grounds
  off-lane cars (`rust/src/nodes/sim/render/car_visual.rs`, `agent_frame.rs`); only buckets that
  changed are uploaded, and a car at rest is not uploaded again. See
  [Render Movement](traffic.md#render-movement).
- There is no level of detail: one mesh at every distance out to 2,500 m, fading over 2,000 to
  2,500 m (`CAR_FADE_START_M`, `CAR_DRAW_DISTANCE_M` in `rust/src/config.rs`).
- Ground support: `agents.gd` finds each model's contact points by scanning its vertices at
  `y = 0` and registers them with `set_vehicle_ground_support`; Rust keeps five profiles
  (`rust/src/nodes/sim/render/vehicle_ground.rs`). The exact solve runs within 250 m of the
  camera and a cheaper one beyond.
- Parked cars are drawn by `godot/scripts/renderers/parked_cars.gd` through the streaming core
  with the same meshes and no lamps; see
  [Parked-car rendering](traffic.md#parked-car-rendering-park-01-park-02).
- Shadows: shadow-only twins near the camera take each bucket's mesh and material.

### Vehicle models and lamps (`RENDER-26`)

Cars are the Kenney models (`godot/assets/models/vehicles/civilian/` sedan, sports sedan, SUV,
luxury SUV; `freight/` delivery van) with their colormap atlas variants. Generic Finnish models
were tried on this branch and withdrawn as not good enough to ship; they are kept off-tree. The
Kenney meshes carry no lamp data, so `vehicle_lamps.gd` holds a measured table of front and rear
lamp anchors per model (left lamps mirror X); procedural cars derive theirs from the AABB.
`vehicle_resources.gd` builds a car's body as one surface (`RENDER-46`): the loader gives every
glTF primitive of a model (the body panels and four wheels) the model's one atlas material, so it
concatenates them in node order with their indices offset; their vertices, triangles and winding
stay the model's own, and a MultiMesh of moving or parked cars draws the body once. The lamp
surfaces keep their own materials. Any model the game loads goes through the same build, so
replacement models need no merged assets; a loader that reads several materials from a model
would merge per material.

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

### The `[vehicle]` pack class

**As built: parsed, with no runtime consumer.** `rust/src/assets/asset/vehicle.rs` reads a
`[vehicle]` section with `vehicle_class` (`civil`, `police`, `fire`, `ambulance`, `utility`,
`bus`), `vehicle_family` (`sedan`, `suv`, `van`, `truck`), `length_m`, `width_m`, `height_m` and
`color_variants` (each a `name` and an `albedo_file`). The manifest also knows `wheel` and `light`
anchor types (`rust/src/assets/asset/model.rs`). Nothing loads, spawns or draws a vehicle asset.
The asset browser lists them under "Vehicles".

The v1 specification text below was moved here from `asset_editor.md` unchanged. Where it and
the parser differ (the family list, the shape of `color_variants`, `asset_class` and `mesh`
fields), the parser is what exists. The [model contract](#model-contract-tools-41) and the
[planned pack work](#planned-pack-assets-drafts-and-the-bundled-pack-tools-41-tools-42) replace
it.

#### V1 vehicle asset contract

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

#### V1 vehicle `asset.toml` schema

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

#### Orientation of imported vehicles

Vehicle compatibility rule:

- Newly imported vehicle assets normalize to canonical `+Z` forward during import and export.
- The hidden runtime compatibility rotation applies only to legacy bundled vehicle content that has not been reauthored yet.

The other orientation and pivot rules for all asset classes stay in
[`asset_editor.md`](asset_editor.md).

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

## Planned: pack assets, drafts and the bundled pack (`TOOLS-41`, `TOOLS-42`)

**Planned.**

Vehicle assets reuse the pack and registry system. The `[vehicle]` section gains:

- `body_class = "car" | "van" | "truck" | "bus"`. `length_m`, `width_m` and `height_m` stay and
  are checked against the class box.
- `spawn_weight`, the model's share inside its class.
- `paints`, a list of colour and weight; a default palette per class when absent. At most 64.
- `seats`.
- `[[vehicle.wheels]]` and `[[vehicle.lamps]]`: typed tables copied from the sidecar. The unused
  `wheel` and `light` anchor types are then removed.
- `[[lods]]`, already in the manifest, lists the four tiers.
- `vehicle_family` gains `hatchback`, `estate` and `bus`. Nothing in the simulation reads it.

Checked when a pack loads, O(vehicle assets): the class box, at least four wheels resting on
the ground, the required lamp groups, the four tiers.

Draft command. `tools/model_draft.py` and `rust/src/assets/authoring/model_draft/` take the
second sidecar kind:

- the sidecar reader accepts version 2 `kind: "vehicle"` strictly, as it reads version 1;
- a vehicle role file gives `pack_id`, `asset_id` (for example `vehicle.civil.estate_compact`),
  `vehicle_class`, and optionally `spawn_weight`, `paints` and `seats`;
- the draft copies the tiers and textures, writes `asset.toml` and the ledger
  `model_sources/<asset_id>.json`, and refuses a model that fails the load checks; a rerun on an
  existing asset reports stale geometry, as for buildings;
- no lot, door or parking rule applies, and the building rules are untouched.

Bundled pack (`TOOLS-42`):

- The own models ship in a bundled, default-enabled pack, `godot/bootstrap/mods/vehicles/`, like
  `parking`, so a fresh install with no user pack has traffic. Other packs may add vehicles; the
  catalogue merges them by class and weight.
- **The pilot pair alone replaces the placeholder cars** (the owner, 2026-11-21). Every car is
  drawn as the compact estate, its variety coming from the per-car paint of the palette; every
  freight vehicle, van or truck, is drawn as the rigid box truck in its `plain` and `haulier`
  variants. There is no wait for more models: the seven fleet models are ordered only after the
  pilot pair has been seen in the engine, and join the pack when they are built.
- Game-side work on `TOOLS-41`, `TOOLS-42` and `RENDER-65` starts when the car-route work
  (`PARK-24`) has landed.
- With no vehicle model at all, the procedural box car of `vehicle_resources.gd` stays as the
  last fallback, sized to the car class box.
- Removed with the pilot pair: `godot/assets/models/vehicles/` (the third-party car models leave
  the shipped game), the lamp table of `godot/scripts/renderers/vehicle_lamps.gd`, the vehicle
  type names that mean models (the freight marker stays), and the car models from the
  placeholder cars' line in `CREDITS.md`; that line keeps the same author's building models,
  which only tests use and release packages leave out. The own models' credit is added.
- Tests and benchmarks that load car meshes use the bundled pack's models.

## Planned: renderer (`RENDER-65`)

**Planned.**

**Catalogue.** A Rust table built from the registry when packs load, O(vehicle assets), sorted
by qualified asset id. Per body class: the models and their weights. Per model: tier files and
switch distances, wheel pivots, ground contact points taken from the wheel facts (replacing the
vertex scan and the fixed five profiles), lamp entries and the paint palette. GDScript receives
it once, loads each tier through Godot's `GLTFDocument` as buildings are loaded, builds the
material and registers the meshes. This closes `RENDER-59`.

**Which model a car gets.** The simulation keeps what it has: vehicle type 0 to 3 means a car,
4 means freight. The drawn model and paint are picked at render time by a stable hash of the
vehicle's saved identity inside its body class. Moving and parked rendering call the same
function, so a car keeps its look across the handoff. No save change; the saved model and colour
bytes are no longer read. Enabling another vehicle pack reshuffles which car looks like what and
changes nothing in the simulation. With the pilot pair alone the car class has one model, so
every car is the estate in its own paint, and every freight vehicle is the truck, its variant
picked by the same hash (the owner, 2026-11-21).

**Per-car data: 16 floats, 64 bytes** (today 12 floats). A transform and one custom-data
vector, the row shape walkers already use:

| Float | Content | Range |
| --- | --- | --- |
| `custom.x` | front wheel steer angle, radians | -0.61 to 0.61 |
| `custom.y` | wheel roll angle, radians | 0 to 2π |
| `custom.z` | one packed whole number: 8 lamp bits, 6 paint bits, 4 blink-offset bits (18 bits, exact in a float) | |
| `custom.w` | reserved | 0 |

Lamp bits: lights on, brake, indicator left, indicator right, reverse, high beam, beacon, one
spare. Hazard is both indicator bits. No per-instance shader uniforms are used.

**Ids, instance colours and paint.** The ids arrive as `COLOR_0` bytes
([Vertex ids](#vertex-ids)). Rules for the game side:

- Vehicle MultiMeshes keep **`use_colors = false`**, moving and parked alike. With
  `use_colors = true` the instance colour is multiplied into `COLOR`, alpha included, before the
  shader's code runs, and the ids cannot be recovered.
- Paint therefore goes through the instance **custom data**, as the palette index in `custom.z`.
- The shader decodes in `vertex()` and passes the result on in a varying (smooth or flat both
  work, since a triangle's corners are identical):

  ```glsl
  vec4 ids = round(COLOR * 255.0); // x part, y surface, z paint (0 or 255)
  ```

- Ids never travel in an 8-bit or half-float custom channel: `ARRAY_CUSTOM0` as RGBA8 unorm and
  the half-float custom formats are broken in the Compatibility renderer (float custom formats
  are exact).
- Any reader that takes colours as floats rounds them before they become bytes: the engine
  converts a float `ARRAY_COLOR` to bytes by truncation, so a float id that is slightly low
  loses one.
- If the hand-written GLB reader of `vehicle_resources.gd` is still in use when ids arrive, it
  carries the `COLOR_0` bytes into `ARRAY_COLOR` through `Color8`, `_merged_body_arrays` carries
  colours, and the pipeline prewarm template's vertex format (`RENDER-43`) adds the colour
  format.

Tested 2026-11-21 on Godot 4.7.2 with Forward+, Mobile and Compatibility, on one AMD GPU on
Linux:

- the stock glTF import keeps the `COLOR_0` bytes exactly in `ARRAY_COLOR` (stored RGBA8 unorm),
  with no sRGB conversion at import or in any of the three renderers;
- the decode above recovers every id exactly over the full range 0 to 255, on a mesh and on a
  MultiMesh with `use_colors = false`;
- the stock importer drops `_METRUM_IDS`;
- a MultiMesh instance colour destroys the ids (not recoverable in Forward+ or Mobile).

Not tested: other GPUs and drivers, Windows, macOS, and the web export.

**One body shader**, with the cloud shadow (`standard_light.gdshaderinc`) and the draw fade
(`agent_draw_fade.gdshaderinc`):

- Vertex stage: a wheel vertex and its normal are rotated about that wheel's centre, spin about
  `X`, then steer about `Y` when the wheel steers. Wheel centres are a small uniform array per
  model. Body vertices take one branch and no maths. The lamp gain of the vertex's surface id is
  computed from the lamp bits, the global night value and, for indicators, a blink of 90 flashes
  a minute, and passed flat to the fragment stage.
- Fragment stage: albedo × paint (the palette colour where the paint weight is 1; white for a
  `"paint": false` variant), ORM, optional normal, and emission × lamp gain. Glass ids get a gain
  of zero, so glass draws as painted.
- The blink reads a global shader clock that stops when the game is paused, and a per-car
  offset. It touches no simulation state and needs no upload: a car waiting to turn keeps
  blinking while its row stays untouched.

**Tier buckets.** The snapshot already measures each posed car's distance to the eye. It picks
the tier from fixed bands, and the bucket key becomes `model * 4 + tier`. One comparison chain
per posed car, no new pass. A car that changes bucket at a tier switch keeps its smoothed pose.
Shadow twins take each bucket's own tier mesh.

**Parked cars** use the same meshes and shader with all lamp bits zero. Their rows grow to 16
floats for the paint index. Buckets fall from model × colour to model × tier, with the tier per
chunk by the visibility ranges the parked layer already sets: no work per frame.

**Forward axis.** Models load unrotated and the runtime points `+Z` along travel; the 180 degree
turn at load and the `-Z` lamp maths end. The beam shader and the headlight pool read lamp
entries from the catalogue.

**Cost.** The CPU cost per frame stays O(drawn cars), with rows one third larger to upload for
buckets that moved. Wheels and lamps are GPU work per vertex and per pixel. Triangles in view
fall by about ten times at city scale (estimate: about 1 M against 13 M for 6,000 cars in view).

## Planned: brake lights, indicators and steered wheels (`RENDER-66`)

**Planned.** Everything here is computed for posed cars only (those in view and within draw
distance), is a pure function of simulation state read under the snapshot's lock, allocates
nothing per car, and is never written back to the simulation.

| Signal | Source | Computed in | Bound |
| --- | --- | --- | --- |
| Brake | the car's speed fell since the last snapshot, or it stands on a live lane | the frame job, with one remembered speed per drawn car | O(drawn cars), 4 bytes each |
| Indicator, junction turn | the connector the car will take (`planned_next_connector` in `rust/src/simulation/economy/agents/tick/lane_nav.rs`) and which way it bends | the snapshot's pose step | O(1) per posed car near its lane end |
| Indicator, lane change | an active lane change and the side of the target lane | the snapshot | O(1) |
| Indicator, parking | the trip phase (pulling in or out) and the side of the bay or driveway | the snapshot | O(1) |
| Steered wheels | the path's curvature: two tangent samples half a wheelbase apart (`rust/src/nodes/sim/render/lane_pose.rs`) | the snapshot gives the target; the frame job eases toward it | two more lane samples per posed car within 100 m |
| Blink | the shader | | none on the CPU |

Rules:

- A connector's bend side is left, right or straight (25 degrees between its first and last
  tangent). It is stored on the lane when connectors are built; derived, never saved.
- A car signals from 40 m before its lane end, or from its brake look-ahead, to the end of the
  connector.
- Indicators and steering are computed within 100 m of the eye; beyond it the bits are zero.
- Parked cars show no lamps.
- The test that leaves a resting bucket untouched compares the new inputs too: a resting car
  uploads nothing, and a changed lamp bit uploads once.
- Braking is read from the speed change, one snapshot late; the simulation stores no
  acceleration.

## Planned: beams, later lamps and glass (`RENDER-67`)

**Planned, after `RENDER-66`.** The lamp regions are in the models from the pilot on.

| Lamp | On when |
| --- | --- |
| `daytime` | the car is driving, by day; dimmed at night |
| `low_beam`, `tail` | driving, and the global night value is above zero |
| `high_beam` | lights on, and the road has no street lights, and no vehicle is ahead in its lane within 150 m, and none is oncoming on the road's opposite lanes within 300 m |
| `reverse` | a truck reversing in a yard |
| hazard (both indicators) | a truck standing at a dock |

- The high-beam rule is cosmetic: no speed, route or headway reads it. It is decided per posed
  car in the snapshot from the road's street-light flag and two lane-bucket lookups, O(log cars
  in a lane) ahead and O(opposite lanes × log) oncoming, only for posed cars on unlit roads at
  night. Vehicles beyond the current road are not seen.
- To stop flicker at the thresholds the frame job keeps one bit per drawn car, with on and off
  distances of 150 / 180 m ahead and 300 / 350 m oncoming.
- Wheel spin: the roll angle advances by the drawn displacement over the wheel radius, in the
  frame job's existing loop.

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
