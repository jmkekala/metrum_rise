# Credits

## Authors

The people below wrote Metrum Rise and hold the copyright in it.

- Juha Kekalainen
- Veeti Reis

Copyright © 2026 the Metrum Rise authors.

Metrum Rise is free software under the
[GNU General Public License, version 3 or later](LICENSE.md) (`GPL-3.0-or-later`).

The project was licensed under `GPL-2.0-only` until 2026-11-20. On that date all authors
agreed to relicense it under `GPL-3.0-or-later`.

## Original assets

Models, textures and icons made for Metrum Rise are original work. Models and textures are
under CC BY 4.0 unless the folder says otherwise; the scripts that generate them are code,
under `GPL-3.0-or-later`. Each asset folder has a `credits.txt` with the exact terms:

- [Road signs](godot/assets/models/road_signs/credits.txt): sign faces are public domain
  (Republic of Finland, via Wikimedia Commons); the geometry is original.
- [Street lights](godot/assets/models/street_lights/credits.txt)
- [Trees](godot/assets/models/vegetation/trees/credits.txt)
- [Shrubs, hedges and flowers](godot/assets/models/vegetation/landscape/credits.txt)
- [Rocks](godot/assets/models/vegetation/rocks/credits.txt)
- [Road textures](godot/assets/textures/road/finnish/credits.txt)
- [Vehicles](godot/bootstrap/mods/vehicles/pack.toml): the bundled `vehicles` pack's cars and
  trucks, built in the Metrum Rise models repository; CC BY 4.0, as its `pack.toml` states.
- [Tree impostor atlases](godot/assets/textures/vegetation/credits.txt)

## Third-party assets

- Kuopio map: contains data from the National Land Survey of Finland, Kuopio height model and
  Topographic Database, retrieved 2026-04-19. Licence: CC BY 4.0.
- Kuopio map, 18,432 m import (`tools/import_dem_world_definition.py`): contains data from the
  National Land Survey of Finland Elevation model 2 m (`Korkeusmalli 2 m`, 25 sheets around
  Kuopio, retrieved 2026-04-19) and Topographic Database hydrography (lakes, river areas and
  shoals from the open topographic data service, retrieved 2026-04-17). Licence: CC BY 4.0, the
  National Land Survey's open data licence; the downloaded files carry no licence text, so this
  was not confirmed from the files themselves.
- [Kenney](https://kenney.itch.io/): building models used by tests only, not in release
  packages. Licence: CC0 1.0.
- [Quaternius](https://quaternius.com/): character models. Licence: CC0 1.0.
- [ambientCG](https://ambientcg.com/view?id=DaySkyHDRI059B): Day Sky HDRI 059 B. Licence: CC0 1.0.
- [CGBookcase](https://www.cgbookcase.com/textures/grass-01): Grass 01. Licence: CC0 1.0.
- [Poly Haven](https://polyhaven.com/). Licence: CC0 1.0.
  - [Concrete Layers 02](https://polyhaven.com/a/concrete_layers_02)
  - [Asphalt 04](https://polyhaven.com/a/asphalt_04)
  - [Clean Asphalt](https://polyhaven.com/a/clean_asphalt)
  - [Withered Grass](https://polyhaven.com/a/withered_grass)
  - [Dark Rock](https://polyhaven.com/a/dark_rock)

## Third-party software

- [Godot Engine](https://godotengine.org/): MIT licence.
- [godot-rust (gdext)](https://godot-rust.github.io/): MPL-2.0.
- [SQLite](https://sqlite.org/): public domain.
- Rust crates under the MIT, Apache-2.0, BSD, Zlib and MPL-2.0 licences. The complete list
  with versions is in [`rust/Cargo.lock`](rust/Cargo.lock).

Release packages carry the full licence texts of the engine, its components and the crates in
`THIRD-PARTY-LICENSES.txt`. The source code of every release is at
<https://github.com/jmkekala/metrum_rise>, under the release's tag.
