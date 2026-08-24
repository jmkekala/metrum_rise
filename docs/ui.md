# Metrum Rise — UI Architecture

## Paradigm

Hybrid: **top menu bar** for global actions and window launchers + **floating windows** for
information panels and selection-driven properties + **bottom toolbar** for placement tools.

All UI is built procedurally in GDScript. The six `.tscn` files (`MainMenu.tscn`, `Main.tscn`,
`AssetEditor.tscn`, `EconomyEditor.tscn`, `WorldEditor.tscn`, `Router.tscn`) describe the scene node tree but
contain no UI component definitions — all panels, buttons, and windows are constructed at
runtime in `_ready()`. There are no `.tres` theme resource files; style constants are
centralised in `UIStyle` (see below) instead. `WorldEditor.tscn` follows the same procedural-UI
rule as the gameplay and other editor shells.

GDScript is a thin rendering and input bridge only. No simulation logic or game decisions
belong here. Rust methods are called through `SimulationNode`.

### Surface presence per scene

| Surface | MainMenu | Main (gameplay) | AssetEditor | EconomyEditor | WorldEditor |
|---------|:-:|:-:|:-:|:-:|:-:|
| Top menu bar | — | ✓ | ✓ | ✓ | ✓ |
| Bottom toolbar | — | ✓ | — | — | ✓ |
| Context panel | — | — | — | — | — |
| Floating windows | ✓ | ✓ | — | — | — |

The top menu bar is shared across gameplay and editor scenes. AssetEditor and EconomyEditor remain
self-contained editor applications with no bottom toolbar. WorldEditor is the editor-shell
exception: it uses a bottom toolbar because terrain and later water authoring tools belong on
that surface rather than in the top menu.

MainMenu is the startup exception: it is a dedicated front-door surface and does not instantiate
gameplay UI or a gameplay world.

The top menu in editor scenes carries a reduced item set. AssetEditor and EconomyEditor use
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
- `New Game` opens a file picker rooted at `user://worlds/`
- `Load Game` opens a file picker rooted at `user://saves/`
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
| Options | Open the shared options window |
| Quit | Exit the application |

### 1. Top Menu Bar

**Node:** `MenuBar` anchored `PRESET_TOP_WIDE`, height ~28 px.
**Script:** `scripts/ui/top_menu.gd` *(implemented)*.

| Menu   | Items |
|--------|-------|
| File   | New Game, Save `[Ctrl+S]`, Load `[Ctrl+L]`, —, Options, —, Quit |
| View   | Overlays submenu (None `[7]`, Pollution `[8]`, Noise `[9]`, Desirability `[0]`, Deposits `[-]`), — , Toggle Zoning Overlay |
| City   | City Statistics *(window)*, Economy Overview *(window)*, Demand Overview *(window)* |
| Tools  | Open Asset Editor, Open Economy Editor |
| Help   | Keyboard Shortcuts *(window)*, About |

The menu bar owns global save/load/quit actions. These are currently handled as keyboard
shortcuts in `input_manager.gd`; the keyboard shortcuts remain but the menu provides the
discoverable entry point. Shortcut-bearing items are currently rendered as plain bracketed
text in the label itself (for example `Save [Ctrl+S]`) rather than using a separate
accelerator column.

Gameplay `New Game` still opens a file picker rooted at `user://worlds/` and loads the selected
`WorldDefinition` into the live scene.
Gameplay `Save` and `Load` open file pickers rooted at `user://saves/`.

`top_menu.gd` is attached by each scene root (`Main`, `AssetEditor`, `EconomyEditor`,
`WorldEditor`).
It is not owned by `main_ui.gd`, because the editor scenes do not use the gameplay HUD.

### Options Window

**Node:** draggable Godot `Window`.
**Script:** `scripts/ui/options_window.gd` *(implemented)*.

The same options window is launched from `MainMenu` and from gameplay `File -> Options...`.
It uses a left category rail and right content pane with footer-level `Reset Defaults`,
`Cancel`, and `Apply` actions. General options state is persisted through
`user://settings.cfg` via `scripts/core/game_settings.gd`; the window currently remembers its
last active category plus size and position. The same settings file also stores reusable
`layout/<id>` sections for player-adjusted floating-window sizes, positions where appropriate,
and split-panel offsets such as Economy Overview's budget/service/policy panes. `Accessibility`
owns the runtime-safe `UI Scale`
setting, currently bounded to `80%..150%` in `5%` increments and applied immediately to
scale-aware procedural UI labels/buttons, including gameplay HUD, Options, Building Inspector,
and Economy Overview detail surfaces. Scale-aware floating windows also declare base/default
sizes through `UIStyle`, so window defaults and minimums grow with Accessibility scale and
gently grow on high-resolution viewports while preserving user-resized larger windows and restored
layout values. `Graphics` owns the Apply-based `Fullscreen` display toggle, persisted through
`user://settings.cfg` and applied through the Options footer and on boot. `Mods` embeds the content-pack manager from
`scripts/ui/pack_manager.gd`; pack selection changes are persisted through
`user://active_packs.cfg` and take effect after restart. The previous gameplay toolbar `Mods`
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

**Nodes:** Procedural bottom-strip layout rooted under a full-screen `Control`. The left
gameplay HUD is an `HBoxContainer` of fixed-height `PanelContainer` shells. The right-side
tool menu uses a unified outer group `PanelContainer` plus an inner fixed-height toolbar-row
shell so the submenu stack can read as one cluster while the actual toolbar row still matches
the clock / city-status / RCI strip height.
**Script:** `scripts/ui/main_ui.gd` (current — stays here).

The toolbar is the primary tool-selection surface. It is always visible during gameplay.

| Button  | Activates |
|---------|-----------|
| Roads   | Road sub-menu (Walkway, 2-Lane, 4-Lane, One-Way, Cul-De-Sac) + draw-mode options (Straight / Spline) |
| Zoning  | Zoning sub-menu with one always-visible lower row for Rect / Brush plus Residential / Commercial / Industrial family buttons; the profile row above is collapsed by default and opens only after clicking a family button, which also selects that family's first profile |
| Services | Service-building asset palette for explicit civic / utility placement |
| Industry | Explicit resource-extractor asset palette; selecting an extractor temporarily shows the Deposits overlay through building placement and extraction-polygon drawing; placing a mine switches to extraction-polygon drawing with a pale-blue cursor sphere under the mouse and a live filled preview of the current polygon; the first vertex must start within 10 m of the building footprint and becomes a stronger light-blue close marker, edges may not cross, closing only happens by clicking that first vertex again, the committed polygon is still validated against the building link distance, and committed coal pits render as a terrain-shader coal-texture mask |
| Terrain | Terrain sub-menu for gameplay terrain tools |
| Inspect | Activates `SelectTool`; clicking a building while `SelectTool` is active opens the Building Inspector window |

Bulldoze is intentionally separate from the centered construction toolbar: a bottom-right icon
button activates `BulldozeTool`. Hover asks Rust for exactly one target, prioritizing building
footprints before road edges; left click deletes one target; right click or Esc exits the tool.

Sub-menus expand upward above the toolbar row, the same as today. In the current
implementation the right-side menu cluster has:
- one outer translucent group wrapper around the whole menu stack
- one transparent inner layout shell for the actual toolbar row
- fixed-height bottom HUD shells for the clock, city-status panel, and R/C/I meter

Gameplay HUD status widgets: the clock / simulation-speed panel remains anchored at bottom-left,
a compact city-status panel sits immediately to its right, and the R/C/I demand meter sits to
the right of that. The city-status panel shows current treasury value plus live agent count,
and refreshes continuously so paused-state edits such as road building still update the value
immediately. The demand meter shows Residential / Commercial / Industrial net growth pressure
as three vertical bars using the same green / blue / yellow palette as the zoning families.
Each bar is centered on a zero baseline and spans `-100%..100%`. All four bottom-strip surfaces
currently use the same white `HUD_TEXT_SIZE` typography for their primary labels.

Zoning interaction rule: `Rect` and `Brush` are shared paint-mode controls, not their own
submenu. Opening the Zoning tool shows only the lower row. Clicking `Residential`,
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
| Z          | Toggle Zoning tool |
| M          | Toggle Move tool |
| V          | Toggle Select tool |
| C          | Toggle Cul-De-Sac tool |
| Y          | Toggle Terrain Sculpt |
| Space      | Pause / unpause |
| Escape     | Cancel active tool |
| Ctrl+Z     | Undo |
| F12        | Add 1,000,000 money and lock R/C/I demand at 100% |
| 7 / 8 / 9 / 0 / - | Overlay modes |

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

---

### 4. Floating Windows

Floating windows are Godot `Window` nodes. They use the engine-provided title bar, drag
behavior, and close button, can be open simultaneously, and are dismissed via that close
button or programmatically.

Most windows are instantiated on first open and hidden (not freed) on close so state is
preserved within a session. The Building Inspector is the exception: it creates one window
per inspected building so multiple inspectors can stay open at once, and each is freed when
its close button is used.

| Window | Launcher | Script / status | Content |
|--------|----------|-----------------|---------|
| Options | MainMenu `Options` or gameplay `File -> Options...` | `scripts/ui/options_window.gd` *(implemented)* | Shared options shell with category rail, content pane, footer-level apply/reset/cancel actions, and persisted window state through `user://settings.cfg`. |
| Graphics | Options → Graphics | `scripts/ui/graphics_options.gd` *(implemented)* | Embedded fullscreen/windowed display toggle, persisted through `user://settings.cfg` and applied through the Options footer. |
| Accessibility | Options → Accessibility | `scripts/ui/accessibility_options.gd` *(implemented)* | Embedded UI Scale control, persisted through `user://settings.cfg` and applied immediately to scale-aware procedural UI fonts and eligible floating-window sizes. |
| Building Inspector | Click building with no active tool or while `SelectTool` is active | `scripts/ui/building_inspector.gd` *(implemented)* | Per-building stats: type, level, occupancy, budget, revenue, inventory, extraction-pit reserve/depletion, alerts. Multiple building windows may be open simultaneously; clicking the same building again closes that building's inspector, and visible inspector windows refresh on each in-game hour boundary. Uses Godot's built-in draggable `Window` chrome. |
| Road Properties | Select one or more road edges with `SelectTool` | `scripts/ui/road_properties_window.gd` *(implemented)* | Edge class (Standard / Bridge / Tunnel), No Buildings flag, and slope warnings for the current selection. Uses Godot's built-in draggable `Window` chrome. |
| Mods | Options → Mods | `scripts/ui/pack_manager.gd` *(implemented)* | Embedded content-pack browser / manager panel. |
| City Statistics | City → City Statistics | inline placeholder in `scripts/ui/top_menu.gd` | Placeholder window for future population, housed/unhoused counts, budget summary, and utility status. |
| Economy Overview | City → Economy Overview | `scripts/ui/economy_overview.gd` *(implemented)* | Live fiscal budget, services, and policy overview with persisted window layout and split-panel offsets. |
| Demand Overview | City → Demand Overview | inline placeholder in `scripts/ui/top_menu.gd` | Placeholder window for future residential/commercial/industrial demand pressure, spawn credit, and growth candidates. |
| Keyboard Shortcuts | Help → Keyboard Shortcuts | inline in `scripts/ui/top_menu.gd` *(implemented)* | Read-only shortcut reference. |

Windows that display live simulation data call `SimulationNode` methods each time they are
opened. They do not hold Rust-side state. The Building Inspector additionally refreshes any
visible inspector windows on each in-game hour boundary; other live windows should stay
snapshot-on-open unless there is a clear need for an explicit low-frequency refresh path.

---

### 5. Overlays

Overlays are rendered as shader-driven mesh layers on the terrain. They are toggled via the
View menu or keyboard shortcuts `7`, `8`, `9`, `0`, and `-`.

Current implementation detail: `terrain.gd` exposes five overlay modes through
`overlay_mode` — `0=None`, `1=Pollution`, `2=Noise`, `3=Desirability`, `4=Deposits`.
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
  TEXT_ALERT    = Color(1.00, 0.40, 0.30)
  HUD_TEXT_SIZE = 16
  ZONE_RESIDENTIAL = Color(0.20, 0.45, 0.25, 0.75)
  ZONE_COMMERCIAL  = Color(0.20, 0.34, 0.62, 0.75)
  ZONE_INDUSTRIAL  = Color(0.55, 0.47, 0.14, 0.75)

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
  HUD_STRIP_HEIGHT = 60.0   # fixed shell height for clock / city-status / RCI / toolbar row
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
    cul_de_sac_tool.gd
    economy_editor.gd
    economy_graph_canvas.gd
    editor_camera_input.gd
    input_manager.gd
    inspect_tool.gd
    lane_tool.gd
    launch_router.gd
    main_ui.gd
    move_tool.gd
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
    EconomyEditor.tscn
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
      move_tool.gd
      select_tool.gd
      cul_de_sac_tool.gd
      lane_tool.gd
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
    editors/                      Asset and economy editor screens
      asset_editor.gd
      economy_editor.gd
      economy_graph_canvas.gd
      analyze_assets.gd
      world_editor.gd
      world_editor_camera_input.gd
    ui/                           HUD, menus, floating windows, style
      ui_style.gd                 Style constants and StyleBoxFlat factory methods
      main_ui.gd                  Bottom toolbar, sub-menus, clock/speed panel
      city_status_panel.gd        Compact treasury + agent-count HUD panel
      demand_meter.gd             Compact R/C/I HUD demand meter
      top_menu.gd                 MenuBar + PopupMenu tree, menu action dispatch
      options_window.gd           Shared Options window category shell
      graphics_options.gd         Embedded Options -> Graphics settings panel
      accessibility_options.gd    Embedded Options -> Accessibility settings panel
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
core/input_manager.gd   Tool state, keyboard shortcuts, camera, Ctrl+S/L save/load
editors/world_editor.gd WorldEditor shell, tool UI, world load/save flow, camera policy owner
editors/world_editor_camera_input.gd  WorldEditor camera input wrapper and UI-capture gating
SimulationNode.get_demand_pressures  HUD-only R/C/I meter source, returns normalized -1.0..1.0 display pressures
SimulationNode.get_treasury_balance  HUD treasury source, snapshot-backed for paused-state edits
SimulationNode.get_agent_count       HUD live-agent source, snapshot-backed
```

`top_menu.gd` dispatches File/City/View/Help actions by calling `InputManager` methods or
opening windows directly. It does not own simulation state. Each scene root attaches its own
instance; `main_ui.gd` remains gameplay-only.

Camera ownership rules:

- gameplay and WorldEditor share one world-camera core in the `CameraNode` native node
- shared world-camera behavior includes:
  - orbit math
  - pan math
  - zoom semantics
  - focus-on framing
  - terrain-clearance clamping so gameplay and WorldEditor cameras never go under terrain or
    inside hills
- gameplay `input_manager.gd` and `editors/world_editor_camera_input.gd` are input-routing
  wrappers only; they decide when UI owns input and which scene-local camera policy applies
- zoom bounds, far clip policy, and focus padding may differ between gameplay and WorldEditor as
  explicit scene policy
- AssetEditor keeps its own separate sandbox camera controller because it is not terrain/world
  constrained and uses a different viewport layout

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
