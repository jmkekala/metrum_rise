# Metrum Rise — UI Architecture

## Paradigm

Hybrid: **top menu bar** for global actions and window launchers + **floating windows** for
information panels and selection-driven properties + **bottom toolbar** for placement tools.

All UI is built procedurally in GDScript. The five `.tscn` files (`MainMenu.tscn`, `Main.tscn`,
`AssetEditor.tscn`, `WorldEditor.tscn`, `Router.tscn`) describe the scene node tree but
contain no UI component definitions — all panels, buttons, and windows are constructed at
runtime in `_ready()`. There are no `.tres` theme resource files; style constants are
centralised in `UIStyle` (see below) instead. `WorldEditor.tscn` follows the same procedural-UI
rule as the gameplay and other editor shells.

GDScript is a thin rendering and input bridge only. No simulation logic or game decisions
belong here. Rust methods are called through `SimulationNode`.

### Surface presence per scene

| Surface | MainMenu | Main (gameplay) | AssetEditor | WorldEditor |
|---------|:-:|:-:|:-:|:-:|
| Top menu bar | — | ✓ | ✓ | ✓ |
| Bottom toolbar | — | ✓ | — | ✓ |
| Context panel | — | — | — | — |
| Floating windows | ✓ | ✓ | — | — |
| Debug console | ✓ | ✓ | ✓ | ✓ |

The top menu bar is shared across gameplay and editor scenes. AssetEditor remains a
self-contained editor application with no bottom toolbar. WorldEditor is the editor-shell
exception: it uses a bottom toolbar because terrain and later water authoring tools belong on
that surface rather than in the top menu.

MainMenu is the startup exception: it is a dedicated front-door surface and does not instantiate
gameplay UI or a gameplay world.

The top menu in editor scenes carries a reduced item set. AssetEditor uses
File plus editor-specific menus. WorldEditor uses File plus Help. AssetEditor and WorldEditor have
no `Return To Game` action and no City / Demand / Economy launchers, which are gameplay concepts.

---

## Surfaces

### 0. Main Menu

**Node:** full-screen `Control`.
**Script:** `scripts/core/main_menu.gd` *(implemented)*.

MainMenu is now the default normal-launch surface. Normal startup must not instantiate a gameplay
map before the player chooses content.

Current deterministic rules:

- normal launch routes to `MainMenu.tscn`
- startup creates `user://worlds/`, `user://mods/`, and `user://saves/`, then copies missing
  bundled starter entries from `res://bootstrap/worlds/` and `res://bootstrap/mods/`
- when `user://settings.cfg` is missing, startup seeds default general UI/runtime settings
- when `user://active_packs.cfg` is missing, startup seeds it with the bundled `kenney` pack
  enabled; a saved empty enabled-pack list remains an explicit player choice
- `MainMenu` contains no `SimulationNode`
- `MainMenu` contains no terrain, water, road, or gameplay HUD surfaces
- `New Game` opens the save browser's world list (`user://worlds/`), then the New Game dialog
- `Load Game` opens the save browser's save list (`user://saves/`)
- selecting a world or save stores the request in the `LaunchState` autoload and then enters
  `Main.tscn`
- `Main.tscn` must consume that pending request on startup and load the selected world/save before
  gameplay continues
- if gameplay is opened without a pending request and without benchmark-style command-line flags,
  it returns to `MainMenu.tscn`

MainMenu v1 actions:

| Action | Result |
|--------|--------|
| New Game | Pick a `WorldDefinition` from `user://worlds/` and enter gameplay |
| Load Game | Pick a city save from `user://saves/` and enter gameplay |
| World Editor | Spawn a `--world-editor` instance |
| Options | Open the shared Options overlay |
| Quit | Exit the application |

### 1. Top Menu Bar

**Node:** `MenuBar` anchored `PRESET_TOP_WIDE`, height ~28 px.
**Script:** `scripts/ui/top_menu.gd` *(implemented)*.

| Menu   | Items |
|--------|-------|
| File   | New Game, Save `[Ctrl+S]`, Load `[Ctrl+L]`, —, Options, —, Quit |
| View   | Overlays submenu (None `[7]`, Pollution `[8]`, Noise `[9]`, Desirability `[0]`, Deposits `[-]`), — , Toggle Zoning Overlay |
| City   | City Statistics *(window)*, Economy Overview *(window)*, Demand Overview *(window)* |
| Tools  | Time of Day submenu, Weather submenu (Automatic, Clear, Fair Cumulus, Overcast, Storm), Looks submenu (an hour and a weather in one click), —, Open Asset Editor |
| Help   | Keyboard Shortcuts *(window)*, About |

The menu bar owns global save/load/quit actions. These are currently handled as keyboard
shortcuts in `input_manager.gd`; the keyboard shortcuts remain but the menu provides the
discoverable entry point. Shortcut-bearing items are currently rendered as plain bracketed
text in the label itself (for example `Save [Ctrl+S]`) rather than using a separate
accelerator column.

Gameplay `New Game` opens the save browser's world list and loads the selected
`WorldDefinition` into the live scene. It resets the camera to the new world's centre and initial
orbit, clears the previous city's save filename, and pauses after rebuilding the scene.
Normal play and debug inspection use the same terrain-anchored focus, initial framing, movement,
and zoom. Debug only extends the orbit pitch to look upward from beneath terrain; the focus
continues to follow the ground while the camera may pass below it in that upward view.
Gameplay `Save` and `Load` (also Ctrl+S, Ctrl+L and the pause menu) open the save browser on
`user://saves/`.

The save browser (`scripts/ui/save_browser.gd`) replaces Godot's `FileDialog` for saves and
worlds. It is drawn like Options: the dimmed screen, one centred 760 px column, the menu it opened
from faded out. It lists one folder's `.sqlite` files only, newest first, each as a row with its
name without the extension, when it was saved ("Today 08:03", "Yesterday 21:10", else the date)
and its size. There is no path bar and no folder browsing; `Open folder` shows the folder in the
file manager. Clicking a column header (Name, Saved, Size) sorts by it, Name A-Z and the others
largest first; clicking it again reverses it, and the active header shows its arrow. A search
line sits under the title: in Load and New Game it holds the keyboard from the start, so typing
filters at once, and it stays barely drawn (a faint magnifier and placeholder) until it holds
text, then lights with the accent underline. The search is fuzzy (`scripts/ui/fuzzy_match.gd`):
the typed letters must appear in the name in order, case-insensitive, not necessarily together,
and matches at word starts and in runs rank higher, so `bigint` finds `big-intersection` first.
A search lists the best match first, with the matched letters in the accent colour; clicking a
header then sorts the matches by that column. Esc clears a search before it cancels. Load and
New Game select the top row, also after each keystroke, so Enter takes the best match; a click
selects, a double click or Enter confirms, Up/Down move through the shown rows. Save has a `Save as` name field filled with the current save's name:
clicking a row fills it in, a name that matches an existing save turns the button into `Replace`,
and an empty name or one with `: / \ ? * " | % < >` disables it with the reason on the status
line. Esc cancels. While it is open, keys and the wheel stay with it (TopMenu reports it as an
overlay), and it hands its caller the file's native path.
City and world-definition saves commit and close an adjacent temporary SQLite file before
replacing the destination. A failed write leaves the previous save intact.
Both city-save and world-definition replacement reset the speed display to paused and close
building inspectors anchored to the previous world. The first pause toggle then resumes play.
Speed requests update the controller/HUD only after the Rust API accepts the command into its
queue. Non-finite values and speeds above 32× are rejected; finite negative values retain pause
clamping. The controls read the existing ascending steps from Rust's clock, which also owns
save-speed validation. The `simulation_speed_input_test.gd` regression is externally time-bounded
because both infinity and huge finite multipliers stalled the old clock loop (`AUDIT-01-C10/C11`).
Asset-editor camera input also calls the shared Rust `CameraNode` directly, with editor-specific
bounds and framing; it keeps no second orbit/transform implementation.

City snapshots also store the active `CameraNode` focus point, yaw, pitch, orbit distance, and
projection in the same SQLite transaction as the world (format version 59). Loading restores
those controls immediately after world replacement, before terrain/water residency refreshes;
the next pan, orbit, or zoom therefore continues from the saved view. The saved focus height
already includes terrain clearance and is restored without another terrain adjustment. Versions
57/58 and simulation-only snapshots have no camera state and keep the scene's current view.
Loading an upward debug view in normal mode clamps its pitch to the normal downward limit.

`top_menu.gd` is attached by each scene root (`Main`, `AssetEditor`, `WorldEditor`).
The menu is plain uppercase text down the left side over a flat dark fill (a gradient this dark
banded): the title with a
short accent rule, then the entries, dim at rest and white with an accent bar at their left
edge under the mouse (`UIStyle.menu_text_button`). There is no card and no button boxes. The
pause menu uses the same layout over the dimmed city.

The mouse's back (thumb) button acts as Esc in the menus (`UIStyle.is_mouse_back`): it closes
Load Game, the New Game world browser and dialog, the in-game Save and Load browsers and Options;
in the pause menu it dismisses the leave confirmation, else resumes. Outside the menus it does
nothing.

It is not owned by `main_ui.gd`, because the editor scenes do not use the gameplay HUD.

### Options

**Node:** `CanvasLayer` overlay (layer 120) drawn in the game's own UI over a dimmed screen.
**Script:** `scripts/ui/options_menu.gd` *(implemented)*.

The same Options overlay opens from `MainMenu`, gameplay `File -> Options...` and the pause
menu. It is one centred column, 760 px wide at 100% UI scale (at most 92% of the screen),
with no card around it: the `OPTIONS` title, two text tabs underlined in the accent colour
while active, `Display` (Screen, Graphics and Interface sections) and `Mods` (installed packs),
then the active tab's rows, which scroll. Placeholder categories are gone until they have
settings. Every setting is a row (`options_rows.gd`): its name at the left, its value in a
fixed 240 px column at the right, a hairline below, the whole row lit with an accent edge under
the mouse. A hovered row's description shows in a hint line above the footer. Choices are
`‹ VALUE ›` cyclers: the arrows step back and forth and wrap, clicking the value steps forward,
and On/Off settings are two-value cyclers, so the overlay opens no drop-down popups. UI scale
is a short slider with its percentage. The footer is text: `Reset Defaults` (the active tab's
panels), the status, `Cancel` and `Apply` in the accent colour. Esc and Cancel discard pending
edits, and the overlay takes the keyboard while open (the camera stays put, and the wheel
scrolls the page instead of zooming, as under the pause menu). The menu it opened from (main
menu or pause menu, group `covered_by_options`) fades out while it is open, so its text does
not show through beside the rows. General options state is persisted through `user://settings.cfg` via
`scripts/core/game_settings.gd`; Options remembers its last tab. The same settings file also
stores reusable `layout/<id>` sections for player-adjusted floating-window sizes, positions where
appropriate, and split-panel offsets such as Economy Overview's budget/service/policy panes.
Interface (`accessibility_options.gd`) owns the runtime-safe `UI Scale`
setting, bounded to `80%..200%` in `5%` increments and applied immediately to
scale-aware procedural UI labels/buttons, including gameplay HUD, Options, Building Inspector,
and Economy Overview detail surfaces. Scale-aware floating windows also declare base/default
sizes through `UIStyle`, so window defaults and minimums grow with the UI scale and
gently grow on high-resolution viewports while preserving user-resized larger windows and restored
layout values. They also refit whenever the viewport they open on is resized (`UI-02`): leaving
fullscreen shrinks the game window, and a window sized for the fullscreen viewport would otherwise
hang off its edges with its footer out of reach. The refit keeps the player's size where it fits,
holds it to 92% of the viewport and moves it back inside. `Graphics` owns Apply-based `Fullscreen`, `V-Sync` (default on), `Show FPS`
(a frame rate and frame time readout at the right end of the gameplay menu bar, `scripts/core/fps_overlay.gd`), `Max FPS`
(Unlimited/30/60/120/144; the main menu always holds `120` or the lower player cap, and nothing else sets the cap outside benchmarks: a world starting up once reset it to 60, `UI-06`), `Render scale` (Native, or 77/67/50% upscaled with FSR 2, which
also turns off the screen-space AA pass), `Shadows` (High, the default; Medium: High's soft
filter and penumbra over two unblended cascades instead of four, `RENDER-15`; or Low: plain PCF, two
cascades, no cascade blending; stored as High 0, Low 1, Medium 2, so a settings file from before
Medium keeps its choice), `View distance` (Full, or Reduced: the far plane floors at 3 km instead of
9 km) and `Building detail` (Performance/Balanced/Quality; default Balanced), persisted in the
`graphics` section of `user://settings.cfg`. Apply takes effect live: frame settings through
`GameSettings.apply_display_settings()`, shadows and view distance through the
`graphics_settings` group (`SceneLighting`, `InputManager`), building detail through the
building renderers without reimporting assets. Cancel discards pending edits and Reset
proposes defaults. Detail uses the shared screen-size LOD policy, not asset-authored distance
bands; it changes neither simulation nor visibility. Gameplay benchmarks
(`--gameplay-road-benchmark`) ignore the player's render settings and measure the defaults.
These replace the former `METRUM_GFX` preset and its F8-F11 hotkeys.
Display settings apply through the Options footer and on boot. `Mods` embeds the content-pack manager from
`scripts/ui/pack_manager.gd`; pack selection changes are persisted through
`user://active_packs.cfg` and take effect after restart. Packs list as rows in the page's own
scroll (a nested scroll area there collapsed to zero height); a pack the running city has
loaded cannot be replaced or removed while Options is open in game. The previous gameplay toolbar `Mods`
button is intentionally removed so content-pack management lives under global options rather than
construction tools.

WorldEditor menu:

| Menu | Items |
|------|-------|
| File | New World, Open World, Save, Save As, Quit |
| Help | Keyboard Shortcuts, About |

Deterministic rule:

- terrain and water authoring tools do not live in the WorldEditor top menu
- WorldEditor does not expose `Return To Game`

AssetEditor menu:

| Menu | Items |
|------|-------|
| File | New Asset, Save, Quit |
| Asset | Reload Packs, Import Mesh |

Deterministic rule:

- `New Asset` clears the right-side pack/asset/building fields, active preview mesh parts, and mesh part list
  while preserving the explicit comparison ghost
- AssetEditor does not expose `Return To Game`

---

### 2. Bottom Toolbar

**Nodes:** Procedural layout rooted under a full-screen `Control`. The category row stays
centred at a fixed bottom inset, with independent submenu panels above it and road options
to the left. `top_status_bar.gd` places the compact status row at top-left, below the actual
scaled top menu height. Both surfaces register with `debug_console_keep_clear`; no status
collision calculation lifts the bottom toolbar.
**Script:** `scripts/ui/main_ui.gd` (current — stays here).

The toolbar is the primary tool-selection surface. It is always visible during gameplay.

The category row's tiles are 50 px (`HudIcons.dock_button`), their icons filling 74 % of the
face: painted full-colour pictures for Roads, Zoning, Services (a wrench), Industry (a mine
cart) and Terrain (`assets/ui/icons/dock/`, never tinted, so selection shows in the tile fill
and underline only), and Bulldoze as the line bulldozer in the alert red the road builder's
remove button uses. The `u01` demo slice uses hand-authored white SVG tool icons elsewhere, one
`UIStyle.HUD_ACCENT`, 72 px road tiles and names on hover. `hud_icons.gd` owns the icon/tile factories and 0.12 s
ease-out hover/press feedback; `hud_tooltip.gd` shows wrapped name/detail/cost cards after
150 ms. `hud_road_thumbnail.gd` paints Rust's `parts` and `part_widths_m` through the existing
road-part glyphs. Built-ins occupy a five-column grid; a `+` tile opens the builder and the
fixed-width custom menu tile previews the selected custom layout. Round `u02` extends the same
helpers to zoning family/profile tiles, segmented workflow/shape choices and the parcel gear,
plus service class and asset tiles (`zoning_panel.gd`, `asset_panel.gd`). Unknown pack classes
use a generic building glyph. Tooltips show available asset metadata; the asset bridge does
not currently expose cost/upkeep. `road_options_view.gd` owns the road toggle layout. InputManager emits
tool transitions for persistent marks; no new per-frame callback is added. Accessibility
refreshes the fonts and icon ink size; tile footprints remain bounded at 1280×720.
Godot parse-only checks passed for the changed scripts and affected tests; runtime regressions
and in-game visual review remain pending. Zoning control sync uses signals instead of polling.

Road guiding lines and the G shortcut are removed. The road-options icon toggle controls
zoning-grid snapping; road-connection snapping remains available. Distance and angle
measurements appear only during a placement. See [road interaction](roads.md).


| Button  | Activates |
|---------|-----------|
| Roads   | Road sub-menu: the usable road types from Rust (`ROAD-101`, `scripts/ui/road_type_menu.gd`: built-ins Walkway, 2-Lane Road, 4-Lane Road, One-Way 1L, One-Way 2L, then one "Custom roads" drop-down listing the player's custom types, a separator and "Road builder..." (opens the Road Builder window); the active type marked; a tile's hover card (`scripts/ui/road_type_card.gd`) gives the name beside its speed limit sign, symbol chips for the lanes (arrows), width, median swatch, parking sign, turn pockets (arrows and length, accented) and only the unusual sidewalk, street light and zoning cases, then Rust's cost with the money sign; key 7 cycling the custom types flashes the card of the type it lands on; a type with turn pockets shows a turn-arrow badge at its tile's top right; the drop-down items keep a text tooltip with the layout, speed, sidewalk side, street lights, zoning, pockets and Rust's cost), then the Roundabout tile (key O, `ROAD-62`) and Edit roads (`SelectTool`, V: click a built road for Road Properties, a junction for the Junction window; a building opens the Building Inspector); with a left-side icon options panel (`scripts/ui/road_options_view.gd`): Tool mode (Straight / Curve, the simple curve's three clicks / S-curve, four clicks / Freeform, clicked nodes then an edit phase: drag nodes and handles, Ctrl breaks a joint, Enter or a click elsewhere builds, `ROAD-112` round N; then Continuous: each road starts in the direction the last one ended, round CT), Snapping (roads, guide lines, zoning grid), Structure (Straight grade, T: bridges and tunnels where the grade leaves the ground), Steps (angle lock and length step, Shift draws free) and, while the roundabout tool is on, Diameter (a slider in 1 m steps over the range Rust gives for the site, with the size and lanes read-only, "36 m, 1 lane"; the two keys right of P, `[` `]` on a US layout and Å ¨ on a Nordic one, step 2 m, held at the ends). O opens the Roads menu when it is closed. The tool's outline is blue when it would build, red with the reason when not; see [`roads.md`](roads.md#roundabouts-road-62) |
| Zoning  | Zoning sub-menu in one row: the cell/parcel tool options on the left, then Residential / Commercial / Industrial family tiles; a family's profiles (low, medium and high density each with its own silhouette) open to their right after clicking the family, which also selects its first profile |
| Services | Service-building asset palette for explicit civic / utility placement (class tiles, then the open class's asset tiles, each with the asset's manifest `icon` or else the class icon; see [`asset_editor.md`](asset_editor.md#v1-common-assettoml-fields)); beside a road side without a sidewalk the preview turns red and the click is refused with "buildings need a sidewalk on their side of the road" (`ROAD-109`), as in Industry |
| Industry | Explicit resource-extractor asset palette; selecting an extractor temporarily shows the Deposits overlay through building placement and extraction-polygon drawing; placing a mine switches to extraction-polygon drawing with a pale-blue cursor sphere under the mouse and a live filled preview of the current polygon; the first vertex must start within 10 m of the building footprint and becomes a stronger light-blue close marker, edges may not cross, closing only happens by clicking that first vertex again, the committed polygon is still validated against the building link distance, and committed coal pits render as a terrain-shader coal-texture mask |
| Terrain | `scripts/ui/terrain_panel.gd`, laid out like Services: one row of category tiles, Raise and lower (the sculpt tool), Trees, Shrubs, Flowers, Hedges and Rocks, then Remove (E, in red, an uprooted sapling so it does not read as the bulldozer), and beside them the open category's planting options as picture tiles. Each picture is a file, `assets/ui/vegetation/<name>.png`, baked offline by `tests/bake_vegetation_pictures.gd` (run headful: `godot --path godot --script res://tests/bake_vegetation_pictures.gd`) from Rust's `get_vegetation_preset_sample` (one plant, a stand of seven for a mix, or a four-module hedge row) drawn with the game's meshes in a private world with fixed light, so weather and wind never show in it; re-bake when a preset or a plant mesh changes, and `vegetation_edit_test` fails on a missing file. A category tile shows one of its options' pictures. Names, and what a mix plants, are in the tooltips. Opening a category plants its first option (or the current one if it is in it); Shift + wheel steps through every option and opens the category it lands in. See [`terrain.md`](terrain.md) |

Bulldoze sits after a separator at the right end of the category row. Its icon button
activates `BulldozeTool`. Hover asks Rust for exactly one target, prioritizing building
footprints before road edges; left click deletes one target; right click or Esc exits the tool.
A ring is one target (`ROAD-62`): the hover shows the whole ring and names it beside the pointer,
"Roundabout, 4 arms".

**Demolition previews** (`ZONE-09`-`ZONE-11`, branch `roads-over-zoning`; rules in
[`zoning.md`](zoning.md#roads-road-edits-and-placements-over-zoned-land-zone-09-zone-10-zone-11)).
Roads, road edits and explicit placements take the zoned land they cover. Every tool that can
take land outlines the lots it would remove with the bulldoze hover's red fill and line
(`scripts/renderers/highlight_mesh.gd`, extracted from `bulldoze_tool.gd`; unshaded, from Rust's
`demolish_lots`, four corners per lot, built or empty) and names the buildings on them from Rust's
`demolish_count`; plotted land (a placed building's lot, a farm field, a mine's extraction area)
turns the preview red with its reason and outlines nothing. Wording, all "demolishes N
building(s)" unless noted:

| Tool | Where the count shows |
| --- | --- |
| Road tool | the HUD measurement line, after the cost |
| Roundabout tool | the label beside the ring |
| Road Properties | the selected roads' widenings outlined; Apply asks "Apply removes N building(s). This cannot be undone." |
| Junction window | the pocket row's verdict ("..., demolishes N building(s)") and the row's take outlined |
| Services and Industry building placement | the price label; while the core is busy the hover keeps its last ghost, outline and label |
| Industry extraction polygon | the quote label; the close click takes the land |
| Edit Field | the hint line; refusals name the plotted land ("field overlaps a placed building's lot") |

The outline colour and the wording are the user's decisions, not yet taken (built with the
bulldoze red and the texts above). Undo of a road edit restores the road and the paint, never the
buildings; placements have no undo (`ZONE-12`).

Road previews expose full readiness only for the exact current pointer input with a staged
canonical road/terrain pair. Coarse ribbons, retained older poses and missing resident terrain
resources remain visibly provisional (`terrain preview pending`). Cancelling or invalidating a
paired preview restores the original road and terrain resources without changing simulation state.
The complete backend and presentation contract is owned by [`roads.md`](roads.md#preview-commit-undo-and-load); `ROAD-68-E` removes the coarse ribbon and provisional older poses.

Sub-menus expand upward above the toolbar row. The menu cluster has:

- one transparent layout wrapper around the menu stack
- separate translucent panels for the category row and road controls
- a status row centred inside the menu bar (it joins the bar's layer) for the clock, speed,
  city status and R/C/I bars

Gameplay HUD status widgets share that row. Speed is pause/resume (Space), slower, the current
speed ("4x" or "Paused") and faster, stepping through Rust's existing steps with the existing
handler. Zoning and Services keep one palette row each: opening a zone's profiles or a service
class widens the row instead of stacking another panel above it. Tooltips name the control and
show its shortcut as a keycap in a compact label; data tooltips (road types, assets, profiles)
keep their figures in a wider card. Show FPS draws its readout at the right end of the gameplay
menu bar. Esc with nothing left to cancel or close opens the pause menu (`pause_menu.gd`,
owned by TopMenu): it stops the clock and restores the speed on Resume, and offers Save, Load,
Options, Main Menu and Quit; leaving asks first, as nothing tracks unsaved changes.
Keyboard tools open their dock panel (R, X, C and V the Roads panel, Y the Terrain panel) and
switching the tool off closes it. Tab, or a click on the demand bars, slides in the City panel
from the left: treasury, agents and per zone its project count, Rust's growth figures as icon
chips (named on hover; zero free lots warns) and the constraint notes the figures cannot show.
While Roads, Zoning or Terrain is open, 1-9 act on its tiles, each showing its digit in a small disc at
its bottom-right corner (`HudIcons.set_number`; `MainUI.handle_number_key`); a key press plays
the tile's click animation (`HudIcons.activate`). Roads: the five built-in types, new
custom road (6 again closes the builder; digits typed in the builder reach the palette unless a
text field has focus), the custom-roads tile (each press picks the next custom type), Roundabout, Edit
roads. Zoning: Residential, Commercial, Industrial (again: that zone's next profile), the
cells/parcels switch (its digit sits on the workflow it switches to), the four cell shapes,
Erase. The zone palette sits left of the cell tools so the digits read left to right. Terrain:
Raise and lower, Trees, Shrubs, Flowers, Hedges, Rocks (again: that category's next option),
Remove.
The city-status panel shows a money icon, current treasury value and a small live agent count,
and refreshes continuously so paused-state edits such as road building still update the value
immediately. The growth meter shows exact Residential / Commercial / Industrial project counts
from the last hourly assessment: newly selected, previously queued, and physically constructing.
The green / blue / yellow bars share a scale whose maximum is the largest current count; they are
not percentages. Three tinted tracks remain visible at zero; the exact counts and all three
explanations are available in the shared styled tooltip. Rust supplies housing need, ready homes, live funded
jobs, missing assets, free/unsuitable lots and customer/income/workforce rejections from recorded
planner decisions. Reading the HUD never runs another city scan or changes a reservation. Until
the first hourly assessment after creation/load, the tooltip explicitly says it is waiting.
F12 enables the growth cheat; the tooltip labels bypassed economic checks rather than inventing
100% demand. `growth_meter_test.gd` covers the bridge, bar scaling, zero counts and explanations.

Zoning interaction rule: Cells/Parcels and Cell/Marquee/Fill/Brush are segmented icon choices;
Erase is an independent toggle. The gear and Free tile only appear in the parcel workflow.
Opening the Zoning tool shows the controls and zone-family row. Clicking `Residential`,
`Commercial`, or `Industrial` toggles the profile row for that family above it and
automatically selects the first profile in that family. Clicking the same family again
collapses that profile row.

WorldEditor toolbar rules:

- WorldEditor uses a bottom toolbar with the same bottom-of-screen placement and the same
  `UIStyle` shell language as the gameplay toolbar
- this toolbar is the primary tool-selection surface for world authoring
- terrain tools live here, not in the top menu
- WorldEditor terrain tools are `Raise`, `Lower`, `Level`, `Smooth`, and `Slope`
- selecting `Raise`, `Lower`, `Level`, `Smooth`, or `Slope` opens an upward-expanding terrain brush row above the bottom toolbar shell
- that terrain brush row owns the shared `Diameter m` and `Strength` controls for all terrain brushes
- active terrain brushes must show their diameter directly on the map with a visible brush preview
- `Slope` captures two world-space anchor points before brushing:
  - the first click captures the slope start
  - the second click captures the slope end
  - after both anchors are present, normal terrain brushing applies the captured grade
- `Water` lives on the same toolbar surface as terrain sculpting
- the current water tool set is:
  - `Lake Fill`
  - `Open Water`
- these water tools are world-editor authoring tools, not gameplay HUD tools
- water tools must not be exposed as top-menu actions
- world-editor water authoring must not begin with a freehand water-depth paint brush
- `Resources` lives on the same toolbar surface as terrain and water authoring
- the current resource tool set is:
  - `Coal`
  - `Erase`
- selecting `Resources` opens an upward-expanding tool row with `Diameter m` and `Richness %`
- authored coal is rendered through the terrain overlay texture; it must not use terrain-following
  meshes or decals that can fight the terrain depth buffer
- WorldEditor does not include gameplay-only toolbar actions such as Roads, Zoning, or Inspect
- WorldEditor does not include gameplay HUD widgets such as the clock, city-status panel, or
  R/C/I meter

WorldEditor water toolbar behavior:

- selecting `Water` opens an upward-expanding tool row above the bottom toolbar shell, the same
  visual language used by gameplay submenus
- `Lake Fill` authors one baseline inland-water body seed plus one target flat surface level
- `Open Water` authors one baseline edge-connected water body seed plus one target flat surface
  level
- water authoring belongs on the same bottom-strip workflow as terrain sculpting so authors can
  switch between carving terrain and placing water features without changing editor shells
- committed `Lake Fill` and `Open Water` records, plus any active surface-fill preview, must show
  visible 3D markers in WorldEditor so authored water locations are readable
- these authored-water markers are WorldEditor-only overlays and must not appear in gameplay
- `Lake Fill` and `Open Water` are two-phase actions:
  - first click starts a transient preview
  - the `Surface +m` control adjusts the preview surface
  - the dedicated `OK` button confirms the preview into authored world state
  - the dedicated `Cancel` button or `Escape` dismisses the preview
- `Lake Fill` and `Open Water` preview must represent a flat baseline water surface, not a dynamic
  shallow-water solve
- surface-fill preview is editor-only transient state and is not saved unless confirmed
- invalid surface-fill preview levels must not flood the visible map; they stay preview-only and
  uncommitted until the author adjusts the level or cancels
- `Lake Fill` is only for closed inland basins
- `Open Water` is only for edge-connected water bodies such as coasts, archipelagos, and world-edge
  lakes

Current WorldEditor shortcuts:

| Key | Action |
|-----|--------|
| 1 | Select `Raise` |
| 2 | Select `Lower` |
| 3 | Select `Level` |
| 4 | Select `Smooth` |
| 5 | Select `Slope` |
| 6 | Select `Lake Fill` |
| 7 | Select `Open Water` |
| 8 | Select `Coal` |
| 9 | Select resource `Erase` |
| Left Mouse | Sculpt terrain with the active tool; `Level` captures the clicked height for the stroke; `Slope` uses the first two clicks to capture anchors before brushing; surface-fill tools use first click for preview; resource tools paint the authored coal layer |
| Shift+Left Mouse | Remove the nearest authored water feature for the active water tool; erase coal while the coal brush is active |
| Middle Mouse | Orbit camera |
| Right Mouse | Pan camera |
| W / A / S / D | Pan camera |
| Mouse Wheel | Zoom camera |
| Ctrl+N | New world |
| Ctrl+O | Open world |
| Ctrl+S | Save world |
| Ctrl+Shift+S | Save world as |
| Escape | Cancel active surface-fill preview, otherwise clear active tool |

**Keyboard shortcuts** (owned by `input_manager.gd`, toolbar reflects active tool visually):

| Key        | Action |
|------------|--------|
| R          | Toggle Road tool |
| X          | Toggle Walkway tool |
| O          | Toggle Roundabout tool (the two keys right of P, `[` `]` on a US layout, step its diameter) |
| Z          | Toggle Zoning tool |
| V          | Toggle Select tool |
| Y          | Toggle Terrain Sculpt |
| Space      | Pause / unpause |
| , / .      | Slower / faster (also keypad - / +) |
| Escape     | Cancel active tool; then close the open panel; then the pause menu |
| Ctrl+Z     | Undo |
| F12        | Add 1,000,000 money and enable the private growth cheat |
| 1 / 2 / 3  | Zoning: Residential / Commercial / Industrial (when no palette is open) |
| 1-9        | The open Roads or Zoning palette's numbered tiles |
| 7 / 8 / 9 / 0 / - | Overlay modes (7-9 when no Roads or Zoning palette is open) |

**Camera** (owned by `input_manager.gd`):

| Input | Action |
|-------|--------|
| Middle Mouse drag | Orbit camera |
| Alt+Left Mouse drag (Option on macOS) | Orbit camera, for a touchpad with no middle button |
| Mouse Wheel | Zoom camera, one step per notch |
| Two-finger scroll (touchpad) | Zoom camera, in fractions of a step |
| Pinch (touchpad) | Zoom camera by the pinch factor |

W / A / S / D pan only while no embedded window, such as a dialog or an open menu, is
exclusive or holds focus; that window's own text fields are invisible to the game window's focus
owner, so typing a save name also panned the camera (`UI-01`).

Godot 4.7.1 dispatches each touchpad gesture twice, so `input_manager.gd` consumes the repeated
copy before any handler sees it. The vegetation brush takes Ctrl (radius) and Shift (option) with
the wheel or a two-finger scroll. macOS turns Shift with a vertical scroll into a horizontal one,
so wheel left and right step the brush like wheel up and down.

---

### 3. Selection Windows

Selection-driven gameplay properties now use floating `Window` surfaces instead of a docked
context panel.

Placement rule: selection windows open near the cursor location that triggered them. They
prefer the right side of the cursor with a small padding so the clicked world-space target
remains visible; if that placement would run off the gameplay viewport, they open on the left
side instead.

| Trigger | Window |
|---------|--------|
| Road edge selected (`SelectTool`) | Road Properties window |
| Building clicked with no active tool | Building Inspector window |
| Building clicked while `SelectTool` is active | Building Inspector window |
| Car, bike or pedestrian on a road clicked with no active tool | Agent Inspector window, and the camera follows the agent |

While the `junction_connectors` debug view is on, a click with no active tool within 12 m of a
junction focuses that junction in the overlay instead of opening the inspector. A click elsewhere,
or Esc, clears the focus ([`debug.md`](debug.md#junction-connectors)).

A click with no active tool that hits an agent opens the Agent Inspector instead of the
Building Inspector. A click that misses every agent ends the inspection and then inspects as
usual; Esc and the window's close button also end it. Its home, work and destination links
frame that building and open its Building Inspector. Panning with WASD also stops the follow
without ending the inspection; Follow returns the camera to the agent
([`debug.md`](debug.md#agent-inspector)).

Selection gestures (`AUDIT-01-F7`) are owned by `SelectTool`, including lane connections and
crosswalk controls. The inactive duplicate `LaneTool` and its scene node are removed. Tool
switching and right-click clearing cancel both lane and edge gestures. Release is observed before
GUI/unhandled routing; if a later handler consumes it, deferred cleanup cancels without applying
a world edit. A newer press survives that deferred cleanup. Missing world hits clear the ribbon,
and matching edge/lane IDs only clear the source when the target handle is incoming; an outgoing
self-loop handle still creates a connection. Clicks use their event position. Each press or drag
update resolves the terrain hit once, sharing it across control picking.

**Road highlight and hover.** The selected roads are drawn as one ribbon over the road with
`assets/materials/road_highlight.gdshader` (unshaded, no depth test): a faint fill, a glowing
outline at both edges and, on each vehicle lane, chevrons drifting the way that lane's traffic
goes (Rust's `lanes` in `get_edge_cross_section`: each lane's offset and direction), so a two-way
road shows both directions. Hovering the Road Properties window's parking Left or Right toggle
lights that half of the road, which names the sides the toggles mean. While `SelectTool` is on, the one road under the cursor (not already selected, no button
down, no junction selected, the GUI not hovered) gets the same outline without chevrons and the
cursor turns to a pointing hand, which shows that a click inspects roads. The hover costs one
non-blocking Rust call per frame (`pick_road_piece`: the roadbed's ray walk and the surface probe
under a single `try_lock`; `false` while the simulation thread holds the core, when the hover stays as it
was rather than blinking off), and its one-edge mesh is rebuilt only when the hovered edge
changes; nothing is drawn over the rest of the network.
Picking is exact (Rust's `pick_road_piece(origin, direction, wait)`: the roadbed piece the ray
meets on the visible surface, `Vector2i(road, junction)`), for the hover, the click and a drag
selection alike: a junction's paved piece picks the junction, a road's span (sidewalks included)
or the piece of a node with fewer than three roads picks that road, and anything else picks
nothing, however near a road is. Every junction in view where a click opens the Junction window
(three or more roads, from Rust's `get_junction_markers(centre, half_extent_m)`: a node-grid
rectangle query, `O(nodes in the rectangle × degree)`, empty while the core is busy) carries a
small dot, about 4 px across at any zoom; the hovered and the open junction are outlined around
their paved area (`footprint_halo.gd` around `get_junction_outline`, the junction piece's kerbs
closed across its road mouths), exactly where a click opens it. The square read is centred on the
view, 1.3 × the camera's distance to it (150-900 m; dots are hidden zoomed out past that), and is
read again only when the view moves a quarter of it, zooms by a quarter or the network changes.
The open junction's
lane labels and crosswalk labels ("Zebra on", with "Click to turn off" or the refusal on a second
line) are white or yellow text in a dark outline at a fixed size on screen.

**Check boxes and switches** (`scripts/ui/check_icons.gd`). `MainUI` sets the CheckBox and
CheckButton icons on Godot's default theme (again after a UI scale change; a theme on the root
window would stop at `MainUI`'s CanvasLayer): an unchecked box is
a light outline, an off switch a light outlined track with its knob at the start, and the on
states fill with the HUD accent; disabled ones at 40 % opacity. Sizes match Godot's defaults
(16 x 16, 32 x 16 at scale 1), so layouts measure the same.

`selection_gesture_test.gd` sends real viewport input and verifies command identity/counts,
consumed releases, tool changes, right-click cancellation, self-loop direction, crosswalk precedence,
source clearing and continued ordinary edge dragging. Five assertions fail against the old tool;
the corrected tool passes. The consumed-release fixture explicitly verifies interception by a later
input handler; it does not claim window-system mouse capture or focus-loss coverage.

Five alternating matched headless process pairs use Godot 4.7.2, CPU 0, one Rayon worker and
`METRUM_DEBUG=0`. A fixed two-lane junction runs ten warmups and nine samples of 256 drag updates.
The real tool rebuilds its ribbon against controlled simulation responses; assertions outside the
timer check all 22 vertices, both snapped endpoints and the final connection. Median process
medians are 232.121 → 232.113 µs per update, while terrain-query calls fall from two to one. This
isolates GDScript/mesh construction and establishes no whole-frame or native terrain speedup.
Control lookup remains O(local lane/crosswalk handles), and ribbon geometry has ten fixed segments.

Command: `RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 godot --headless --path godot --script
res://tests/selection_gesture_test.gd -- --benchmark-selection-drag`. The runs use release extension SHA-256 `f3647c04fc3714b8f84a1f2fa506d3d93417bd47292deb4d22ee13e2b1ad605e`. No builds, tests or
source mutations overlapped measurement. The final scene also corrects its resource load-step count;
this metadata is outside the drag fixture.

---

### 4. Floating Windows

Floating windows are Godot `Window` nodes. They use the engine-provided title bar, drag
behavior, and close button, can be open simultaneously, and are dismissed via that close
button or programmatically.

Most windows are instantiated on first open and hidden (not freed) on close so state is
preserved within a session. The Building Inspector is the exception: it creates a fresh window
for each inspected building and frees it on close. Only one inspector is open at a time, so
inspecting another building closes the previous window. With Inspect active, Esc clears the
selection and closes either inspector; a second Esc exits Inspect. V toggles Inspect off,
empty ground clears the selection, and Road Properties' close button clears its highlight.
Both inspectors use `UIStyle.keep_keyboard_with_game` and shared title/section/action styles.

| Window | Launcher | Script / status | Content |
|--------|----------|-----------------|---------|
| Options | MainMenu `Options`, gameplay `File -> Options...` or the pause menu | `scripts/ui/options_menu.gd` *(implemented)* | In-UI overlay with Display and Mods tabs, footer-level apply/reset/cancel actions, and the last tab persisted through `user://settings.cfg`. |
| Graphics | Options → Display | `scripts/ui/graphics_options.gd` *(implemented)* | Fullscreen, V-Sync, Show FPS, Max FPS, render scale (FSR 2), shadows, view distance and building detail, persisted through `user://settings.cfg` and applied through the Options footer. |
| Interface (UI Scale) | Options → Display | `scripts/ui/accessibility_options.gd` *(implemented)* | Embedded UI Scale control, persisted through `user://settings.cfg` and applied immediately to scale-aware procedural UI fonts and eligible floating-window sizes. |
| Building Inspector | Click a building's footprint with no active tool or while `SelectTool` is active (a direct hit through Rust's `pick_building_at`; a click on a road or open ground opens nothing, however near a building) | `scripts/ui/building_inspector.gd` *(implemented)*, card chrome in `scripts/ui/inspector_card.gd` | A borderless card (no OS title bar) with a 3 px top stripe in the building's accent (residential green, commercial blue, industrial amber, a utility the HUD accent). Header: the icon well (its utility service, a farm's field, an extractor's pit, else its zone), an uppercase eyebrow (zone, "Power utility", "Farm"), the name, then a chip row: level pips, the business status pill and a business's "Budget $X" (a home's households are a KPI). Then alerts as tinted banners (broken in red; deserted, rezoning and construction in yellow), then KPI tiles with meters: a business's Workers, Output, Stock (or Used power for a plant) and Today's profit (green or red, yesterday in the tooltip); a home's Households, Residents (an age-stacked meter with a children / adults / elders legend), Supplies and Money. Then pill tabs over dense key-value grids (four columns when every value is short, else two): People (a farm's residents, job slots when some are inactive, utility state and plant funding), Money (only what the header and KPIs leave out: profit yesterday, utility revenue, a home's money in all and lowest supplies, a farm household's money and supplies), Site (extraction-pit reserve/depletion or the farm field with Edit Field), Power (a plant's output and staffed capacity, its own use under an "Own use" heading; for any building "Used yesterday", local supply, imports and cost), Stock (inventory with daily input rates) and Parking (valid places and rejected ones; the tab carries a warning icon when any are rejected). Empty tabs are left out, the last tab picked stays picked for the next building in the session, and the window fits its height to the open tab. The header drags the card and × closes it; it is fitted to its content before it is placed beside the cursor (header labels clip with an ellipsis rather than wrap, so the placement sees the real height). One inspector is open at a time: clicking another building replaces it, clicking the same building again closes it, and the open window refreshes on each in-game hour boundary. While its card is open the building is ringed on the map in the card's accent (`scripts/renderers/footprint_halo.gd`: the road selection's band around Rust's `footprint`, depth-tested so the building hides the far side). It holds the keyboard focus only while a mouse button is down in it, so WASD, zoom and Esc keep moving the camera while it is open. |
| Agent Inspector | Click a car, bike or pedestrian on a road with no active tool | `scripts/ui/agent_inspector_panel.gd` *(implemented)*, controlled by `scripts/renderers/agent_inspector.gd` | The agent's mode, state, speed, activity, buildings, lane, route and latest trip failure (`DEBUG-06`). Home, work and destination link to their buildings. Opens beside the cursor; draggable and resizable `Window` chrome with its size persisted. Like the Building Inspector it holds the keyboard focus only while a mouse button is down in it (`UIStyle.keep_keyboard_with_game`), so camera controls keep working. |
| Road Builder | "Road builder..." in the Roads row's "Custom roads" drop-down | `scripts/ui/road_builder/road_builder_window.gd` *(implemented)* | Builds, edits, duplicates and deletes custom road types (`ROAD-101`) in the game's card chrome: the header is one row, the type's name as an editable field (Rust's suggestion until the player types one, Rust's description as its tooltip) with the first reason the type cannot be created beside it. Below it the library column (built-ins, the player's types, unusable entries with reasons, the library error), the layout strip to scale on a stage that takes the spare height (the road centred between grass verges, a total-width dimension under it and Rust's cost line under that with the money sign before each amount, "15 / m" and "0.10 / m / day" apart, the units telling build cost from upkeep; click a + then a tile, or drag and drop), six part tiles on one row with wrapping names (the sixth a kerbside Parking lane, `PARK-05`), a part popup on one row without a title (a median's style as surface swatches and its width, then move-left and move-right arrows and remove as the bulldozer in red, named by tooltips; a second click on the part closes it; Backspace or Delete removes the part whose popup is open); a part's tooltip on the strip is its name and width; the tiles and the speed signs have none, and the speed signs no label, a check box over each end of the strip: clicking the end turns its sidewalk (left or right as the road is drawn) on or off, a 0.75 m shoulder band drawn at an end without one; the type's attributes on one wrapping row ( Speed as Finnish speed limit signs on Rust's posted limits, 20, 30, 40, 50, 60, 70, 80, 100 or 120 km/h (80 steps to 100; `scripts/ui/speed_sign_picker.gd`: the chosen sign in the middle on a recessed track whose ends, half signs included, fade into the panel; the wheel steps one limit, a click on a side sign picks it; the strip slides and the chosen sign swells by 1.5 % for a moment, and the strip leans a little and eases back when scrolled past 20 or 120), hidden for a footpath; Zoning as an icon toggle, off once by default when a new type reaches 80 km/h and disabled with no sidewalk on either side, `ROAD-109`; Street lights as an icon toggle, on by default) with Cancel and Create road / Save at its right end, the warning with zoning for no sidewalks ("No sidewalks: nothing is built along this road") or per side ("No sidewalk on the right: nothing is built on that side"), `ROAD-109`. Persistent, resizable, non-exclusive `Window` (980 x 600, min 780 x 590); Esc closes, Enter creates. See [`roads.md`](roads.md#the-road-builder-road-101-round-rb2). |
| Road Builder turn pockets (`ROAD-128`) | Roads menu → Road builder | `scripts/ui/road_builder/pocket_controls.gd` | Styled independent Left/Right arrows, a metre stepper using Rust's pocket limits, and a junction-length annotation on the cross-section stage. The controls share the speed sign row; Save stays disabled with Rust's reason while the layout cannot take its pockets. Rust owns the drawn type's pockets: `road_tool_set_road_type` takes them from the library with the rest of the type (GDScript holds no copy and has no setter). The road tool shows each refused junction's `RoadReasonText` reason in a short fading label over it (`road_pocket_notice.gd`, from the commit answer's `pocket_refusals`); the road remains built, and a refusal at a T whose approach lacks that turn is normal. Undo removes the stroke and its pockets and refunds both. See [`roads.md`](roads.md#turn-pockets-road-95). |
| Road Properties | Select one or more road edges with `SelectTool` | `scripts/ui/road_properties_window.gd` *(implemented)* | Edge class (Standard / Bridge / Tunnel), the road type (a picker of the usable types with their speed and Rust's cost per metre, or "Custom layout", matched by sidewalk sides too, left and right as the road was drawn; `ROAD-101`, `ROAD-107`) with Rust's in-place verdict and Apply (`ROAD-80`; "Apply (+1,230)" with the cost difference, and a `ConfirmationDialog` "Apply removes N buildings. This cannot be undone." when the change demolishes: the type's zoning off, or the lots a widening takes, which are outlined (`ZONE-10`); a type that would take a sidewalk from under a building's door is refused with that reason, `ROAD-109`), a Parking lane row with Left and Right switches, each switching one kerb in place (`PARK-05`; a side without a sidewalk refuses, with the reason in the warning line), a Speed limit row with the road's limit as its sign (the road builder's `speed_sign_picker.gd`, one whole sign; hidden when only walkways are selected) that changes only the speed of every selected car road, in place, once the picker rests 0.4 s, so a run of wheel steps is one edit and one undo (a refusal shows in the warning line), No Buildings flag (disabled on roads with no sidewalk on either side, `ROAD-109`) and slope warnings for the current selection (turn pockets are edited from the Junction window). Uses the Building Inspector's card chrome (`inspector_card.gd`: borderless, the header drags it, × closes it), content-sized beside the selection: a "Road properties" eyebrow over the selected road (thumbnail in the icon well, name, lanes; the speed is the sign's), segmented class icons with the No buildings toggle, the Parking lane and Speed limit rows, then "▸ Change road type" unfolds a bounded scrollable road-thumbnail grid, the verdict and accent Apply. The native picker remains its selection/metadata model for existing test drivers. |
| Junction | Click a junction's paved area with `SelectTool` (a dot marks it, above) | `scripts/ui/junction_window.gd` *(implemented)*, rows in `scripts/ui/junction_pocket_row.gd` | The Building Inspector's card chrome (`inspector_card.gd`: borderless, the header drags it, × closes it): a "Junction" eyebrow over "Turn pockets" with the junction icon. Opens beside the click (never over the junction; a moved window stays unless it would cover the click); keeps the game's keyboard so Ctrl+Z undoes with it open; the lane spheres and crosswalk toggles stay in the world. Under "Roads coming in" (`ROAD-95`), one card per incoming car road from Rust's `get_junction_pockets`, clockwise from north: "From west" over the road's type, what its end has as a pill ("No pocket", "Left, 60 m"), Left and Right turn-arrow toggles (a side the road cannot carry disabled with Rust's reason), the length from the junction as a − 60 m + stepper (20-300 m by 10; the wheel steps it, a click on the number types it, Enter), then Rust's verdict and accent "Apply (+N)" only once a request differs; the hovered (else last changed) row's road highlighted cyan from the junction over the length and the row outlined (every control in a row passes the mouse on, so the hover holds across the row). After an Apply or an undo the rows are read again, keeping unapplied requests. Above the rows, a warning when the junction's lane rules leave approach lanes without a turn (Rust's `no_movement`, `ROAD-119`): their count and the fix, read again with the rows on each new render generation (`ROAD-120`). No pocket section at a pocket's lane transition or a node without incoming car roads. The card fits its rows, up to four, more scroll; see [`roads.md`](roads.md#the-turn-pocket-tool-round-r5b2). |
| Mods | Options → Mods | `scripts/ui/pack_manager.gd` *(implemented)* | Embedded content-pack browser / manager panel. |
| City Statistics | City → City Statistics | inline placeholder in `scripts/ui/top_menu.gd` | Placeholder window for future population, housed/unhoused counts, budget summary, and utility status. |
| Economy Overview | City → Economy Overview | `scripts/ui/economy_overview.gd` *(implemented)* | Live fiscal budget, services, and policy overview with persisted window layout and split-panel offsets. |
| Demand Overview | City → Demand Overview | inline placeholder in `scripts/ui/top_menu.gd` | Placeholder window for a future expanded view of the HUD’s project counts and concrete growth constraints. |
| Keyboard Shortcuts | Help → Keyboard Shortcuts | inline in `scripts/ui/top_menu.gd` *(implemented)* | Read-only shortcut reference. |

Windows that display live simulation data call `SimulationNode` methods each time they are
opened. They do not hold Rust-side state. The Building Inspector additionally refreshes its
open window on each in-game hour boundary; other live windows should stay
snapshot-on-open unless there is a clear need for an explicit low-frequency refresh path.

Farm inspectors provide **Edit Field** for committed polygons. `field_edit_tool.gd` displays
the existing vertex handles, previews each drag through Rust validation, and commits on release.
Accepted releases refresh the field overlay and merge Rust's updated area, worker capacity and
production ratio into the open inspector immediately. Rejected moves restore the last accepted
shape and display the conflict. A drag over zoned land outlines the lots it takes and the hint
names the buildings; the release demolishes them (`ZONE-11`, no undo). Escape/right-click finishes editing; switching tools or loading
a world also cancels an unfinished drag. No field geometry or economy decisions live in GDScript.
During initial field drawing and resizing, a yellow outline shows the farm lot and a red outline
with translucent fill marks the building/yard footprint the field must avoid. A visible legend
explains the colors. These guides persist until drawing or editing ends and use the authoritative
Rust site geometry, cached once when the tool opens.
See [`economy.md`](economy.md#field-placement-and-editing-econ-07) for land-use rules.
Industry placement clears pending state on deactivation and stops processing while inactive.
Completing a polygon shares that cleanup while preserving the committed farm; placement and
polygon errors appear in the existing tool notification.

Farm inspectors show **Farm Household** occupancy out of one slot, followed by **Children**,
**Adults**, and **Elders**, and the same household money, supplies, and replenishment details as
residential homes. Business workers and production remain in their own section. Counts describe
the resident family, excluding commuting workers, and refresh with the existing hourly inspector
update. Farm housing is independent of the number of jobs (`ECON-08`).

Settings recovery and scaling (`AUDIT-01-F8`) use the default UI scale for NaN/infinite input.
Finite values retain the 0.8–2.0 clamp and 0.05 steps. On top of this setting, the root
window's `content_scale_factor` follows the screen's backing scale
(`GameSettings.apply_display_scale()`, re-applied on `dpi_changed`), so 100% is the size the
OS draws its own interface: 2x on a Retina Mac, 1x where the OS reports no scale. The 3D view
keeps rendering at the window's full pixel size, and `Camera3D.unproject_position` and mouse
positions both use the scaled coordinates. A windowed launch is sized the same way
(`GameSettings.fit_launch_window()`): the project's 1920x1080 grows by the backing scale and is
held to 90% of the usable screen, centred. Benchmark runs keep the fixed project size. Failed config reads discard the whole
partially parsed state before defaults are installed; valid reads preserve unrelated layout values.
A font/window refresh reads the scale once and passes it through the existing tree traversal and
size helpers. Later operations still read current settings; there is no persistent settings cache.
Traversal remains O(scene nodes), with one config read rather than one per styled control plus two
per sized window. Font rounding/minimum size, viewport bounds and retained larger windows remain.

`ui_settings_test.gd` verifies normalization, reset versus valid-load preservation, invalid-value
saving, nested font/window sizes and repeated refresh after changing settings. Five assertions fail
on the baseline. A separate native ConfigFile probe confirms that parse failure retains earlier
keys (`ui-settings-probe.log`); the regression exercises the shared recovery helper without emitting
an expected engine parse error in the normal suite.

Five alternating matched headless process pairs use Godot 4.7.2, CPU 0, one Rayon worker and
`METRUM_DEBUG=0`. Valid settings with scale 1.25 and 16/256/4,096 labels measure one warmup and nine
refresh samples, excluding setup and assertions. Every label retains font size 16. Median process
medians change from 316/5,122/87,932 µs to 120/1,749/31,565 µs. This is a UI-refresh measurement,
not simulation or whole-frame acceptance. Both styles use the same settings reader on valid data;
the before run loads the original UIStyle script, and the after run uses the corrected script.

Command: `RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 godot --headless --path godot --script
res://tests/ui_settings_test.gd -- --benchmark-ui-scale`. Release extension:
`f3647c04fc3714b8f84a1f2fa506d3d93417bd47292deb4d22ee13e2b1ad605e`. No builds, tests or source
changes overlapped accepted measurement.

---

### 5. Overlays

Overlays are rendered as shader-driven mesh layers on the terrain. They are toggled via the
View menu or keyboard shortcuts `7`, `8`, `9`, `0`, and `-` (an open Roads or Zoning palette
takes `7`-`9` for its tiles).

Current implementation detail: `terrain.gd` exposes five overlay modes through
`overlay_mode` — `0=None`, `1=Pollution`, `2=Noise`, `3=Desirability`, `4=Deposits`.

Pollution, noise and desirability refresh after each published daily settlement while open.
The renderer uses the existing snapshot day, caches successful uploads, and retries failures;
world replacement and resource editing retain explicit invalidation. Updating image pixels
reuses existing material bindings. Pixel sampling uses world-space texture centres and authored
environmental cell spacing. `environment_overlay_test.gd` covers cadence/cache lifecycle and real
RGBA image uploads across world-size changes (`AUDIT-01-F6/G4`).
Environmental intensity controls the terrain blend through the image's alpha channel, and the
texture clamps at world edges. `terrain_overlay_shader_test.gd` verifies actual rendered pixels
(`AUDIT-01-G5`). `./run.sh --test` runs this regression in gamescope's headless backend (since
`ROAD-68-H`; `$DISPLAY` only without gamescope, and the run fails when neither renders). It can also run from a graphical
session with `godot --path godot --rendering-method gl_compatibility --script res://tests/terrain_overlay_shader_test.gd`.
Gameplay keyboard shortcuts expose all five values through `input_manager.gd`; WorldEditor also
selects mode `4` while the coal-resource tools are active.

| Key | Overlay |
|-----|---------|
| 7   | None (default) |
| 8   | Pollution |
| 9   | Noise |
| 0   | Desirability |
| -   | Deposits |

Zoning overlay is a separate mesh toggled with the Zoning tool or `View → Toggle Zoning Overlay`.
Traffic overlay is not currently implemented in the terrain renderer; do not expose it in the
menu until a real renderer path exists.

### 6. Debug Console

The debug console is an autoload (`DebugConsole`) available in every scene and build. The key
left of `1` (`§` on Nordic layouts) slides it down from the top edge, anchored left: a third of
the window wide by default, below the top menu and status bar and above any overlapping
bottom toolbar/palette. Its right edge, bottom edge and bottom-right grip resize it, and the size persists across
restarts. It sits above the other surfaces on canvas layer
120. Gameplay keeps the keyboard until one of the console's two text fields is clicked, and Esc
returns it. Its Channels tab also holds a Visual modes section, one value picker per renderer
debug view (terrain, grass, building sites, pedestrian VAT, terrain residency, junction
connectors), which switches the view at once. [`debug.md`](debug.md#console) owns its behavior and commands.

---

## Style Conventions (`UIStyle`)

**Script:** `scripts/ui/ui_style.gd` *(implemented)*.

`UIStyle` owns the shared color, sizing, and shell helpers used by the current HUD/menu
implementation. Most new UI code calls `UIStyle` factory methods instead of constructing
`StyleBoxFlat` inline. `scripts/ui/main_ui.gd` still contains one local toolbar button-capsule
style and should be migrated when that visual treatment is revisited.

```
Color constants
  BG_DARK      = Color(0.08, 0.08, 0.12, 0.93)   # windows, inspector
  BG_PANEL     = Color(0.10, 0.10, 0.10, 0.80)   # generic panel background
  BG_SUBMENU   = Color(0.15, 0.15, 0.15, 0.70)   # sub-menus
  BG_HUD_SHELL = Color(0.07, 0.07, 0.07, 0.72)   # fixed-height bottom-strip shells
  BG_HUD_GROUP = Color(0.07, 0.07, 0.07, 0.56)   # outer menu-group wrapper
  BORDER_ACCENT = Color(0.30, 0.30, 0.45, 0.60)  # window borders
  TEXT_PRIMARY  = Color.WHITE
  TEXT_DIM      = Color(0.72, 0.72, 0.72)
  TEXT_SECTION  = Color(0.65, 0.65, 0.90)        # section headers
  TEXT_ALERT    = Color(0.96, 0.36, 0.40)        # errors, broken states (sign red lifted for text)
  SIGN_YELLOW   = Color(1.00, 0.808, 0.020)      # warnings
  SIGN_RED      = Color(0.886, 0.118, 0.184)     # removal tools' selected state, warning edges
  SIGN_BLACK    = Color(0.07, 0.07, 0.07)        # text on a yellow plate
  HUD_TEXT_SIZE = 16
  ZONE_RESIDENTIAL = Color(0.20, 0.45, 0.25, 0.75)
  ZONE_COMMERCIAL  = Color(0.20, 0.34, 0.62, 0.75)
  ZONE_INDUSTRIAL  = Color(0.55, 0.47, 0.14, 0.75)

The `SIGN_*` colours are the Finnish traffic sign colours, used only with the signs' meanings:
yellow warns (city panel warnings, the yellow alerts in the building inspector, and the gridlock
and stranded-walker notices as yellow plates with a red edge); red forbids or removes (Bulldoze,
zoning Erase and Free light red when selected, through `HudIcons.set_accent`, and broken states
read in `TEXT_ALERT`).

Corner radii
  CORNER_WINDOW = 8     # floating windows
  CORNER_PANEL  = 12    # generic panels
  CORNER_SUB    = 10    # sub-menu panels, zone buttons
  HUD_SHELL_CORNER = 15 # bottom HUD/menu shells

Padding (MarginContainer constants)
  PAD_WINDOW  = 12      # inside floating windows
  PAD_PANEL   = 15      # generic panel padding
  PAD_INNER   = 8       # inside sub-menu panels
  CURSOR_WINDOW_GAP = 40.0  # horizontal gap from cursor to selection windows
  HUD_SHELL_PAD_X = 15      # inside bottom-strip shells
  HUD_SHELL_PAD_Y = 10

Bottom HUD sizing
  HUD_STRIP_HEIGHT = 60.0   # category toolbar shell height
  HUD_BUTTON_HEIGHT = 60.0  # toolbar button target height before inner-shell clamping
  HUD_BOTTOM_MARGIN = 20.0  # bottom-screen gap
  HUD_PANEL_GAP = 12.0      # gap between bottom-strip shells
  HUD_LEFT_MARGIN = 20.0    # left inset for the gameplay HUD strip

Helper factories
  hud_shell_style()         fixed-height gameplay HUD shell
  hud_group_style()         outer unified menu-group wrapper
  hud_clear_style()         transparent inner toolbar-row layout shell
```

---

## Godot Folder Structure

### Pre-Reorganisation Snapshot

Historical reference only. The flat `scripts/` layout below is the pre-`CODE-02`
organization and should not be used for new work.

```
godot/
  assets/
    materials/                    Shaders (.gdshader)
    models/
      buildings/
        commercial/               Commercial building GLBs + textures
      characters/
        civilians/                VAT meshes, blend source files
    textures/
      general/concrete_layers/
      road/clean_asphalt/
  bin/                            Compiled Rust library (libmetrum_rise.so, .gdextension)
  scenes/                         Four flat .tscn files
    AssetEditor.tscn
    EconomyEditor.tscn
    Main.tscn
    Router.tscn
  scripts/                        23 .gd files, entirely flat
    agents.gd
    analyze_assets.gd
    asset_editor.gd
    building_preview.gd
    buildings.gd
    economy_editor.gd
    economy_graph_canvas.gd
    editor_camera_input.gd
    input_manager.gd
    inspect_tool.gd
    lane_tool.gd
    launch_router.gd
    main_ui.gd
    network_renderer.gd
    network_tool.gd
    pack_manager.gd
    road_tool.gd
    select_tool.gd
    terrain.gd
    water.gd
    zoning_overlay.gd
    zoning_tool.gd
  project.godot
```

### Current Layout

This is the active layout after the `CODE-02` script reorganisation pass.

```
godot/
  assets/
    materials/                    Shaders (unchanged)
    models/
      buildings/
        commercial/               (unchanged)
      characters/
        civilians/                (unchanged)
    textures/                     (unchanged)
  bin/                            (unchanged)
  scenes/                         (unchanged — .tscn files stay flat)
    AssetEditor.tscn
    Main.tscn
    Router.tscn
  scripts/
    core/                         Scene-attached nodes, global orchestration
      game_settings.gd            Persistent user://settings.cfg helper for runtime/UI options state
      input_manager.gd            Gameplay tool state machine, keyboard routing, gameplay camera input routing
      launch_router.gd            Scene routing / startup
      main_menu.gd                Front-door scene UI and launch handoff actions
      editor_camera_input.gd      AssetEditor sandbox camera pan / orbit / zoom
    tools/                        Player-facing placement and editing tools
      road_tool.gd
      select_tool.gd
      network_tool.gd
      zoning_tool.gd
    renderers/                    Thin render bridges — read Rust state, update meshes
      network_renderer.gd
      buildings.gd
      agents.gd
      zoning_overlay.gd
      building_preview.gd
      terrain.gd
      water.gd
    editors/                      Asset and world editor screens
      asset_editor.gd
      analyze_assets.gd
      world_editor.gd
      world_editor_camera_input.gd
    ui/                           HUD, menus, floating windows, style
      ui_style.gd                 Style constants and StyleBoxFlat factory methods
      main_ui.gd                  Bottom toolbar, sub-menus, clock/speed panel
      city_status_panel.gd        Compact treasury + agent-count HUD panel
      demand_meter.gd             Compact R/C/I HUD demand meter
      top_menu.gd                 MenuBar + PopupMenu tree, menu action dispatch
      options_menu.gd             In-UI Options overlay: Display and Mods tabs
      options_rows.gd             Options row, heading, value cycler and slider helpers
      save_browser.gd             In-UI Save / Load / New Game file lists
      fuzzy_match.gd              Fuzzy subsequence scoring for list search
      graphics_options.gd         Options -> Display graphics section
      accessibility_options.gd    Options -> Display interface (UI Scale) section
      pack_manager.gd             Embedded Options -> Mods content-pack panel
      road_properties_window.gd   Floating road-properties editor window
      building_inspector.gd       Floating building stats window manager
  project.godot
```

**Layout maintenance rules:**
- Moving a script requires updating its `res://` path in every `.tscn` that references it
  and in every `preload(...)` call in other scripts. Do this in one dedicated pass per
  subdirectory, not incrementally across unrelated sessions.
- New scripts go into their target subdirectory immediately. Do not add new scripts back to
  the flat `scripts/` root.
- `scenes/` stays flat. Scene files are few enough that a subdirectory adds no value.

---

## Script Ownership Map

```
ui/ui_style.gd          Style constants, StyleBoxFlat factory methods — no simulation calls
ui/main_ui.gd           Bottom HUD/menu strip, submenu stack, unified menu-group wrapper, clock/speed panel, city status panel, HUD demand meter, road properties panel
ui/city_status_panel.gd  Compact treasury-value + agent-count panel — reads snapshot-backed SimulationNode values
ui/demand_meter.gd      Compact R/C/I demand meter — reads SimulationNode demand display values
ui/top_menu.gd          MenuBar, dispatches to InputManager or opens windows; shortcut-bearing entries render bracketed inline labels such as `Save [Ctrl+S]`
ui/building_inspector.gd  Window — building stats, calls SimulationNode.get_building_info_at()
ui/road_properties_window.gd  Window — edge class / no-build editing for SelectTool
ui/pack_manager.gd      Window — mod pack browser
ui/debug_console/        DebugConsole autoload controller, widget view, events Logger — reads MetrumDebug only
core/input_manager.gd   Tool state, keyboard shortcuts, camera, Ctrl+S/L save/load
editors/world_editor.gd WorldEditor shell, tool UI, world load/save flow, camera policy owner
editors/world_editor_camera_input.gd  WorldEditor camera input wrapper and UI-capture gating
SimulationNode.get_growth_overview  Cached hourly project counts, figures, constraint notes and full text; O(1) in city size
SimulationNode.get_treasury_balance  HUD treasury source, snapshot-backed for paused-state edits
SimulationNode.get_agent_count       HUD live-agent source, snapshot-backed
```

`top_menu.gd` dispatches File/City/View/Help actions by calling `InputManager` methods or
opening windows directly. It does not own simulation state. Each scene root attaches its own
instance; `main_ui.gd` remains gameplay-only.

Camera ownership rules:

- gameplay and WorldEditor share one world-camera core in the `CameraNode` native node
- `CameraNode` owns camera snapshot/restore; the native game save/load bridge captures and applies
  its state only during save/load, with O(1) work and no per-tick camera synchronization
- shared world-camera behavior includes:
  - orbit math
  - pan math
  - zoom semantics
  - focus-on framing
  - terrain-following focus and camera clearance in gameplay and WorldEditor, identical in
    normal and debug modes when looking downward
  - debug extends pitch to look upward from beneath terrain, bypassing camera clearance only
    for that upward orbit while keeping the same terrain-following focus
- gameplay `input_manager.gd` and `editors/world_editor_camera_input.gd` are input-routing
  wrappers only; they decide when UI owns input and which scene-local camera policy applies
- both wrappers call `CameraNode.configure_world_camera()` for shared terrain setup and the
  existing native application debug flag; clearance defaults and debug parsing are not duplicated
  in GDScript. Input stays in the wrappers; native frame processing advances presentation motion
- projection and orthographic size updates have one native implementation; changing distance
  bounds updates both the orbit distance and orthographic view immediately
- zoom bounds, far clip policy, and focus padding may differ between gameplay and WorldEditor as
  explicit scene policy
- AssetEditor keeps its own separate sandbox camera controller because it is not terrain/world
  constrained and uses a different viewport layout

### Smooth camera motion — CAM-01

Keyboard pan and zoom use the shared Rust camera's frame
update. Mouse orbit and drag-pan remain immediate. Explicit `focus_on`, save/load and
`set_view_state` remain immediate and discard outstanding keyboard/zoom intent; snapshots
store the **displayed** view, not a destination that the player has not yet reached.

`camera_node/motion.rs` owns a constant-size analytic response state, independent of Godot.
Wheel/pinch increments accumulate on the requested distance, while the displayed distance
approaches it exponentially without overshoot. Keyboard input requests a velocity for one
frame; absent input applies braking. Displacement integrates the entire exponential velocity
curve over the frame instead of multiplying its final velocity by delta. A sub-f32 remainder
preserves small horizontal increments at large world coordinates. Idle updates query no
terrain and change no transform (except one deferred water-clearance check after restoration).
Motion integration is O(1), allocation-free, and uses one camera; no collection or spatial
index is introduced. Active frames reuse the existing terrain/water clearance queries.

The exported response times specify **95% response**, not a delay before movement starts:

| Camera property | Initial value | Meaning |
|---|---|---|
| `zoom_response_s` | 0.15 s | Cover 95% of the requested distance change |
| `pan_acceleration_s` | 0.10 s | Reach 95% of requested keyboard velocity |
| `pan_braking_s` | 0.06 s | Remove 95% of velocity after key release |

Zero selects immediate response. Zoom tails stop within a relative distance error of
1e-6; released pan velocity stops below 0.001 m/s. UI capture, focus loss and world-load
gating call `cancel_motion` to stop at the displayed view immediately. Taking over with
follow or mouse drag discards keyboard velocity. Follow still permits zoom and orbit;
a pan request ends following before the inspector's next update.

All three input wrappers process at priority -300, native camera motion at -200, and
ordinary picking/render bridges at 0. AssetEditor updates its pane-centering offset after
camera motion and before its selection/LOD update. Deterministic offline drivers may call
`advance_motion(delta)` only with the camera's automatic processing disabled.

Fresh validation (2026-11-01): nine Rust camera-related tests pass, including response
integration at 30/60/120/240 FPS. Godot `camera_motion_test` and `camera_save_load_test`
pass with `METRUM_DEBUG=0` and `1`; `agent_inspector_test`, `asset_workspace_test` and
`asset_selection_test` pass. These cover queued zoom, orthographic projection, cancellation,
mid-motion save/restore, bounds, direct orbit/drag, following, water clearance and the three
wrappers moving the camera before picking in the same frame. All Cargo commands use the
original checkout's shared `build-slot.sh` wrapper, including `cargo test --offline --lib camera`
and `cargo clippy --offline --all-targets`. Clippy completes with 59 warnings in untouched
`asset_document.rs` and agent SoA code; no camera warnings.

The fresh release control (base `dc979ad8`, library SHA-256 prefix `5314b34176f8446f`)
reproduces the historical baseline's 60 FPS trace exactly. The candidate library is
`944730da7e0736663464277f76d0ded3012b0cd254620fa73c97607c175dff28`;
its camera and motion source hashes are recorded with the replay in `$ARTIFACTS/metadata.json`.
Repeated candidate 60 FPS traces and the rendered/headless traces match exactly; maximum
endpoint coordinate difference across 30/60/120 FPS is 0.069 mm. At 60 FPS, the first city-pan
frame averages 51.1 m/s rather than 240 m/s, and the first wheel notch moves the eye 18.9 m
rather than 66.7 m. Orbit, explicit framing and follow traces are unchanged. The hill's
vertical response remains terrain-driven; this pass does not damp terrain height.

`camera_motion_benchmark.gd` measures command/update cost separately from replay capture.
Both libraries use the same release profile, lockfile and CPU flags. The fixture pauses a
2048 m blank world, uses four Rayon workers, and excludes setup from timed batches.
Each process warms up 10,000 updates, then records 21 batches of 10,000 updates per case.
Pan reverses every 128 frames; zoom alternates a notch every eight frames; delta is 1/60 s.
Three control/candidate pairs alternate execution order. The control's idle case uses a
no-op native pan call as a dispatch baseline, not an automatically scheduled old update.
These measurements cover one camera and existing clearance queries, not populated-city
rendering or simulation throughput. Run each isolated project with its selected library:

```bash
RAYON_NUM_THREADS=4 METRUM_DEBUG=0 \
METRUM_CAMERA_BENCH_OUTPUT="$ARTIFACTS/camera-cost.json" \
godot --headless --path "$ARTIFACTS/project" --script res://camera_motion_benchmark.gd
```

Fresh unprofiled release results, after compilation completed (range of three process medians,
microseconds per frame):

| Workload | Immediate control | Smoothed candidate |
|---|---:|---:|
| Idle dispatch | 0.422–0.434 | 0.415–0.428 |
| Keyboard pan | 3.769–3.869 | 4.240–4.279 |
| Wheel zoom | 0.571–0.579 | 3.881–3.945 |
| Pan and zoom | 4.246–4.305 | 4.297–4.328 |

Zoom now updates clearance/transform every moving frame instead of only on wheel events;
that expected cost remains below 4 µs per frame in this fixture. Pan adds about 0.4–0.5 µs.
The user accepted the hands-on camera feel before integration into `dev`; the synthetic
checks above establish response and bounded camera cost.

### Whole-map zoom — CAM-02

Gameplay uses a fixed **10 m–40 km** orbit range, independent of map size. The input
bridge sets this scene policy through the existing Rust `set_distance_bounds` API;
the shared Rust response and its distance-dependent far plane remain unchanged.
At the usual 45-degree pitch, the limit is about 28 km above the focus point.
Gameplay enables Rust `set_world_bounds_enabled`: the horizontal focus stays inside the
current map rectangle, while the camera eye may orbit outside it. Keyboard pan, drag,
focus and follow share this constraint. At an edge only the blocked velocity component is
cleared, preserving sliding along the edge and immediate reversal. Bounds use the live
terrain-size snapshot in O(1), without allocations or entity scans; stationary frames do
not query it. Saved off-map pivots are corrected on the first native update after load,
when the simulation borrow has ended. Older zoom distances respect the gameplay limit.
The policy is opt-in; asset and world editors retain their existing navigation.
Asset-editor bounds remain local to the asset scene. Visible cloud layers fade before
the camera crosses them; [terrain.md](terrain.md#14-terrain-site-ground-water-and-lighting-materials-are-runtime-presentation) owns
the rendering details. Stars and moon remain directional sky elements.

Original 60 km validation on `camera/playtest`, 2026-11-01: `camera_motion_test`, `camera_save_load_test`,
`weather_menu_test` and `day_cycle_test` pass. The motion regression checks the full range
on a small world, unchanged close-up reach, far-plane clearance, and projection of all four
corners of a 32 km square at 60 km distance (45-degree pitch, cardinal/diagonal yaw, 16:9).
Rendered sweeps use the actual 500 m test world and local Kuopio world. The latter's metadata
is **18 × 18 km (324 km²)**; a separate 32 km terrain file was not available for this run.
All terrain/water/vegetation settling checks pass at the sampled distances and after returning
to close-up. No new LOD or terrain-residency system was necessary.

The opt-in `camera_overview_playtest.gd` reuses the existing scene-capture settling check.
Set `METRUM_OVERVIEW_WORLD` to a world SQLite and `METRUM_OVERVIEW_OUTPUT` to a fresh
`$ARTIFACTS` directory, then run Godot with `--path godot --resolution 1280x720
--script res://tests/camera_overview_playtest.gd`. `METRUM_OVERVIEW_CLOUDS=1` adds
horizon-facing captures/readbacks across the cloud layers. `METRUM_OVERVIEW_VIDEO_ONLY=1`
instead records a 30 FPS PNG sequence of proportional zoom-out/zoom-in input; video capture
is separate from timing measurements.

Historical 60 km rendering measurements: release library `944730da7e073666`, Godot 4.7.2,
Forward+/Vulkan, RX 7900 XTX, 1280×720, four Rayon workers, paused empty worlds, fair weather
and 10:00 lighting. Each distance settles before 120 timing samples; transition p95 uses the
preceding 90 camera-update frames. GPU medians at 400 m / 30 km / 60 km were
0.498 / 0.183 / 0.186 ms on the small world and 1.267 / 0.904 / 0.844 ms on Kuopio.
The largest sampled transition GPU p95 was 2.077 ms on Kuopio. Gamescope enforced V-Sync,
so the roughly 16.7 ms wall-frame measurements are capped, not maximum-throughput results.
These are empty-world rendering measurements, not populated-city performance acceptance.
The original 60 km extension reused the existing camera binary; the subsequent 40 km
boundary policy changes native camera code. The older measurements above do not validate it.

Current 40 km/boundary validation (`camera/playtest`, 2026-11-01): 10 targeted Rust camera
tests and `camera_motion_test`, `camera_save_load_test`, `agent_inspector_test` pass.
`cargo clippy --all-targets` passes with 59 existing warnings in untouched files and none
in changed camera code. All Cargo commands used the shared build-slot wrapper. The motion
regression covers four edges, braking, immediate reversal, diagonal sliding, drag, deferred
off-map restoration, old 60 km saves, replacement-world dimensions, editor opt-out, close-up
zoom and 18 km corner projection at 40 km. The rendered 18 km Kuopio sweep and sustained
pan against all four edges pass, including return to close-up. The previous 32 km framing
result above belongs to the old 60 km limit; it is not a 40 km framing guarantee.

Matched unprofiled release camera benchmark: control `944730da7e073666`, candidate
`5ae1359da565f066`, Godot 4.7.2 headless, four Rayon workers, isolated identical projects,
paused 2048 m blank world, 21 measured batches × 10,000 frames after a warm-up batch.
The candidate enables gameplay bounds; the control lacks that policy. Idle / pan / zoom /
pan+zoom medians were **0.422 / 4.367 / 4.029 / 4.426 µs** for control and
**0.415 / 4.531 / 4.216 / 4.597 µs** for candidate. The extra active-frame cost is about
0.2 µs, with O(1) complexity independent of city population. Reproduce using the existing
`camera_motion_benchmark.gd`, `METRUM_CAMERA_BENCH_OUTPUT="$ARTIFACTS/results.json"`,
`RAYON_NUM_THREADS=4`, `METRUM_DEBUG=0`, and `godot --headless --path "$ARTIFACTS/project"
--script res://benchmark.gd`; each isolated project contains the same fixture and its selected
release library. Render captures were completed before these matched timing runs.

### Scripted camera feel baseline

`tools/camera_playtest.py` runs the shared native camera API in an isolated synthetic
world, with a 10 m reference grid, building-sized landmarks and a 30 m hill. It covers
close/city pan, wheel steps, constant mouse orbit, hill traversal, focus and follow.
It does not exercise gameplay input routing, populated-city frame pacing, water contact,
or editor UI. The recorded baseline below predates `CAM-01`; rerunning uses the selected
library and the current fixture/API.

Run from the repository root with an explicitly selected GDExtension binary and a fresh
artifact directory (`--video` requires Godot, gamescope and ffmpeg on Linux):

```bash
python tools/camera_playtest.py --library godot/bin/libmetrum_rise.so --output "$ARTIFACTS" --video
```

The runner copies the binary and fixture into an isolated project and user profile. It
records binary/source/fixture hashes and commands in `metadata.json`, per-frame traces
at 30/60/120 fixed FPS, a repeated 60 FPS trace, `summary.json`, and a 22-second,
960 × 540, 60 FPS `camera-replay.mp4`. The replay uses unlit diagnostic geometry and
Godot's Compatibility renderer. Fixed-step traces and PNG capture are **not runtime
performance measurements**. Physics jitter correction is disabled in this fixture so
the requested fixed render delta is preserved. Normal gameplay settings are untouched.

Baseline run (2026-11-01): deployed library SHA-256
`1ddb9699688affb9fbbc4b2d4e92eafd52f0abfda489ac157340dcc9a5a9b922`,
Godot `4.7.2.stable.arch_linux.ed1daf0bf`, four Rayon workers; no new Rust build.
The deployed binary's build commit is unknown; the harness records the source checkout
separately rather than claiming that it identifies the binary. All seven trials passed
finite-pose and ground-clearance checks. Repeated 60 FPS traces matched exactly; the
rendered trace matched the headless trace exactly, and endpoint coordinate differences
between frame rates stayed below 0.3 mm.

| Baseline observation | Measurement / implication |
|---|---|
| Pan starts and stops immediately | At orbit distances 40/400 m, the first active frame reaches 24/240 m/s; no movement after release. |
| Wheel zoom is a single-frame step | First inward notch changes distance 400 → 333.333 m, moving the eye 66.667 m. |
| Constant orbit input is even | 120 horizontal pixels/s produces approximately 20.626°/s yaw, with immediate start/stop. |
| Terrain height directly drives the camera | Crossing the 30 m hill at 48 m/s horizontal speed reaches about 35.127 m/s vertical speed. Clearance remained valid in this fixture. |
| Focus and follow acquisition snap | The chosen focus change moves the eye 345.254 m in one frame; acquiring the follow target shifts it 20 m before tracking its 12 m/s motion. |

First tuning candidates are wheel easing and brief keyboard acceleration/braking. Keep
mouse orbit responsive; assess terrain-height damping and focus/follow transitions
separately. Future changes should retain explicit immediate view restoration and the
camera-before-picking process order. This baseline is diagnostic evidence, not acceptance
of a smoothing implementation.

Camera validation (2026-09-11): targeted Rust tests and the headless
`res://tests/camera_save_load_test.gd` pass, covering shared zoom endpoints, hillside movement,
mode changes, saved underground views and immediate orthographic updates when bounds change.
The suite and a WorldEditor policy smoke test pass under both `METRUM_DEBUG=0` and `1`. Updates
remain O(1), with at most two existing heightmap samples and no new allocations. Before/after
library SHA-256 prefixes: `49a1d0bf1c39` / `8f7f61265fa8`. Normal median times were 1.952–1.980
/ 2.034–2.067 µs per update; debug times were 1.938–1.944 / 1.972–2.023 µs. Both remain about 2
µs per input update.

---

## Migration Notes

These are the concrete changes needed to move from current state to the target paradigm.
They are tracked in the roadmap under `CODE-02` scope.

1. **Add `UIStyle`** — completed. `scripts/ui/ui_style.gd` now owns the shared color,
   radius, padding constants and common `StyleBoxFlat` factory helpers.

2. **Add top menu** — completed. `scripts/ui/top_menu.gd` is instantiated from each scene
   root (`Main`, `AssetEditor`, `EconomyEditor`) and remains separate from `main_ui.gd`.
   Gameplay File/View/City/Tools/Help actions are wired, editor scenes expose reduced
   File/editor-action menus, and the City/Help windows currently use placeholder content
   where full data windows are not yet implemented.

3. **Migrate selection-driven properties to `Window`s** — completed for the Building
   Inspector (`scripts/ui/building_inspector.gd`) and Road Properties
   (`scripts/ui/road_properties_window.gd`). Both now use Godot's built-in title bar, drag
   behavior, and close button while preserving the existing selection/edit APIs underneath.

4. **Add City / Economy / Demand windows** — still pending. The top menu currently opens
   inline placeholder windows from `scripts/ui/top_menu.gd`; replace those one at a time
   with dedicated `Window`-based scripts when the live data views are implemented.

5. **Reorganise `scripts/` into subdirectories** — completed. New UI work starts from the
   `scripts/ui/` subtree and follows the current layout documented above.
