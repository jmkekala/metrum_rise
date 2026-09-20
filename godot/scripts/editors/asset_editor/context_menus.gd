# SPDX-License-Identifier: GPL-2.0-only

## On-demand contextual menu presentation for preview, object lists and the library.
## Captured generation/targets prevent delayed actions from editing a replacement document.
extends RefCounted

const Actions = preload("res://scripts/editors/asset_editor/context_actions.gd")
const Library = preload("res://scripts/editors/asset_editor/library_actions.gd")

var editor: Node
var actions: RefCounted
var library: RefCounted
var popup: PopupMenu
var context: Dictionary = {}
var generation := 0
var _viewport_focus := false
var _commands: Array[Dictionary] = []

func configure(owner: Node) -> void:
	editor = owner
	actions = Actions.new(editor)
	library = Library.new(editor)
	popup = PopupMenu.new()
	popup.name = "AssetContextMenu"
	editor.add_child(popup)
	popup.id_pressed.connect(_pressed)
	editor._session.document.changed.connect(func():
		generation += 1
		popup.hide()
		actions.document_changed()
	)
	for pair in [[editor._view._mesh_part_list, "mesh"], [editor._view._site_anchor_list, "anchor"], [editor._view._site_surface_list, "surface"]]:
		var list: ItemList = pair[0]
		list.gui_input.connect(_list_input.bind(list, pair[1]))

func keyboard_context() -> bool:
	var focus: Control = editor.get_viewport().gui_get_focus_owner()
	if editor._ui_captures_editor_text_input() or editor._cam_input._ui_has_modal_popup(): return false
	if focus in [editor._view._mesh_part_list, editor._view._site_anchor_list, editor._view._site_surface_list]: return true
	if focus != null and editor._view.library.is_ancestor_of(focus): return false
	return _viewport_focus

func handle_right(event: InputEvent) -> bool:
	if event is InputEventMouseButton and event.pressed:
		_viewport_focus = editor._view._preview_view_rect.get_global_rect().has_point(event.position)
	if not event is InputEventMouseButton or event.button_index != MOUSE_BUTTON_RIGHT or not event.pressed:
		return false
	if editor._cam_input._ui_has_modal_popup() or not editor._view._preview_view_rect.get_global_rect().has_point(event.position): return false
	open_preview(event.position)
	return true

func _list_input(event: InputEvent, list: ItemList, kind: String) -> void:
	if not event is InputEventMouseButton or event.button_index != MOUSE_BUTTON_RIGHT or not event.pressed: return
	var index := list.get_item_at_position(event.position, true)
	if index < 0: return
	editor.get_viewport().gui_release_focus()
	_open({"kind": kind, "index": index}, list.global_position + event.position, null)
	list.grab_focus()
	list.accept_event()

func open_preview(mouse: Vector2) -> void:
	_viewport_focus = true
	actions.cancel()
	editor._selection.cancel()
	editor.get_viewport().gui_release_focus()
	var hit := {}
	if editor._session.has_document:
		editor._selection.refresh(mouse, true)
		if not editor._selection.hits.is_empty() and not editor._selection.hits[0].get("occluded", false):
			hit = editor._selection.hits[0].duplicate()
	var ground = editor._project_mouse_to_horizontal_plane(mouse, 0.0)
	_open(hit, mouse, ground)

func _open(hit: Dictionary, mouse: Vector2, ground: Variant) -> void:
	_reset()
	if not hit.is_empty(): actions.properties(hit)
	context = {"generation": generation, "hit": hit, "targets": actions.selection() if not hit.is_empty() else [], "ground": ground, "pointer": mouse}
	if not editor._session.has_document:
		_item(popup, "New asset…", "new")
	else:
		if not hit.is_empty(): _objects(hit)
		if hit.get("kind", "") not in ["reference", "ghost"]: _create_menu(ground)
		if hit.is_empty():
			popup.add_separator()
			_item(popup, "Frame entire asset", "frame_all")
			_item(popup, "Hide scale reference" if editor._preview.has_scale_reference() else "Show scale reference", "reference_toggle")
			_item(popup, "Lighting & quality…", "lighting")
		elif editor._session.document.selection_actions(actions._typed(context.targets)).get("delete", false):
			popup.add_separator()
			var count: int = context.targets.size()
			_item(popup, "Delete %d objects" % count if count > 1 else ("Delete mesh part (all LODs)" if hit.kind == "mesh" else "Delete " + ("yard" if hit.kind in ["surface", "vertex"] else "anchor")), "delete")
	_show(mouse)

func _objects(hit: Dictionary) -> void:
	var objects: Array[Dictionary] = actions._typed(context.targets)
	popup.add_separator("%d objects selected" % objects.size() if objects.size() > 1 else actions.label(hit))
	if hit.kind == "reference":
		_item(popup, "Frame reference", "frame")
		_item(popup, "Reset position", "reference_reset")
		_item(popup, "Hide reference", "reference_hide")
		return
	if hit.kind == "ghost":
		_item(popup, "Frame comparison", "frame")
		_item(popup, "Clear comparison", "ghost_clear")
		return
	_item(popup, "Properties", "properties")
	_item(popup, "Frame selection", "frame")
	var caps: Dictionary = editor._session.document.selection_actions(objects)
	if caps.get("rename", false): _item(popup, "Rename…", "rename")
	_item(popup, "Duplicate…", "duplicate", {}, caps.get("duplicate", false), "The main entrance is unique and cannot be duplicated.")
	if caps.get("rotate", false):
		var rotate := _submenu(popup, "Rotate")
		_item(rotate, "Rotate interactively    R", "rotate")
		_item(rotate, "Rotate left 90°", "rotate", {"angle": -90.0})
		_item(rotate, "Rotate right 90°", "rotate", {"angle": 90.0})
		_item(rotate, "Reverse direction 180°", "rotate", {"angle": 180.0})
	if objects.size() != 1: return
	if hit.kind == "mesh":
		var part = editor._parts[hit.index]
		var lod := _submenu(popup, "LOD")
		var panel = editor._view._preview_panel
		context.lod = maxi(0, panel._inspection_lod())
		_item(lod, "Add LOD%d…" % part.lods.size(), "lod_add")
		_item(lod, "Replace LOD%d…" % context.lod, "lod_replace")
		var tiers := _submenu(lod, "Preview")
		_item(tiers, "Automatic", "lod_preview", {"tier": -1})
		for tier in part.lods.size(): _item(tiers, "LOD%d" % tier, "lod_preview", {"tier": tier})
		if part.lods.size() > 1:
			_item(lod, "Remove last LOD — LOD%d" % (part.lods.size() - 1), "lod_remove")
	elif hit.kind == "anchor" and editor._is_main_entrance_anchor(editor._site_anchors_data[hit.index]):
		_item(popup, "Reset to frontage", "entrance_reset")
	elif hit.kind in ["surface", "vertex"]:
		var materials := _submenu(popup, "Surface material")
		for material in ["asphalt", "concrete"]: _item(materials, material.capitalize(), "material", {"material": material})
		if hit.kind == "vertex":
			context.vertex = hit.vertex
			var points: Array = editor._site_surface_vertices(editor._site_surfaces_data[hit.index])
			points.remove_at(hit.vertex)
			var valid: bool = editor._site_surface_polygon_is_valid(points)
			_item(popup, "Delete vertex", "delete_vertex", {"vertex": hit.vertex}, valid, "A yard needs at least three vertices and a non-intersecting outline.")
		else:
			var edge: Dictionary = editor._selection.picker.surface_edge(context.pointer, editor.get_node("CameraNode"))
			if not edge.is_empty() and edge.surface == hit.index:
				_item(popup, "Insert vertex here", "insert_vertex", {"vertex": edge.edge, "point": [edge.point.x, edge.point.y]})

func _create_menu(ground: Variant) -> void:
	if editor._session.params.get("asset_class") != "building": return
	if not context.hit.is_empty(): popup.add_separator()
	var menu := _submenu(popup, "Create")
	var valid := ground != null
	var reason := "Point at the ground to choose a placement position."
	_item(menu, "Import mesh…", "import", {}, valid, reason)
	var entrance: int = editor._main_entrance_index()
	if entrance >= 0:
		_item(menu, "Select main entrance", "entrance_select", {"index": entrance})
	else: _item(menu, "Main entrance", "create", {"kind": "entrance"}, valid, reason)
	for entry in [["Driveway", "driveway"], ["Parking space", "parking"], ["Loading bay", "loading_bay"]]:
		_item(menu, entry[0], "create", {"kind": entry[1]}, valid, reason)
	var yards := _submenu(menu, "Yard surface")
	for material in ["asphalt", "concrete"]: _item(yards, material.capitalize(), "create", {"kind": material}, valid, reason)

func _reset() -> void:
	popup.hide()
	popup.clear()
	for child in popup.get_children(): child.free()
	_commands.clear()

func open_library(mouse: Vector2) -> void:
	_viewport_focus = false
	actions.cancel()
	editor.get_viewport().gui_release_focus()
	_reset()
	var tree: Tree = editor._view._asset_tree
	if tree.is_visible_in_tree(): tree.grab_focus()
	var item := tree.get_item_at_position(mouse)
	var target := {}
	if item != null:
		var metadata = item.get_metadata(0)
		if metadata is String:
			target = {"id": metadata}
			item.select(0)
		elif metadata is Dictionary: target = metadata
	context = {"generation": generation, "library": target}
	if target.has("id"):
		popup.add_separator(editor._asset_browser_label(target.id))
		_item(popup, "Open for editing", "library_open")
		_item(popup, "Create editable copy…", "library_copy")
		_item(popup, "Use as comparison", "library_compare", {}, editor._session.has_document, "Open an asset to use a comparison model.")
		popup.add_separator()
		var location: Dictionary = library.location(target.id)
		_item(popup, "Show in file manager", "library_folder", {}, not location.has("error"), location.get("error", ""))
		_item(popup, "Copy asset ID", "library_id")
		popup.add_separator()
		var trash: Dictionary = library.trash_check(target.id)
		_item(popup, "Move to Trash…", "library_trash", {}, not trash.has("error"), trash.get("error", ""))
	elif target.has("pack"):
		_item(popup, "New asset…" if target.has("type") else "New asset in this pack…", "library_new")
		_item(popup, "Show pack folder", "library_pack_folder")
		_item(popup, "Refresh library", "library_refresh")
	else:
		_item(popup, "New asset…", "library_new")
		_item(popup, "Create pack…", "library_pack_create")
		_item(popup, "Refresh library", "library_refresh")
	_show(tree.global_position + mouse)

func _submenu(parent: PopupMenu, title: String) -> PopupMenu:
	var menu := PopupMenu.new()
	menu.name = "Sub%d" % parent.get_child_count()
	parent.add_child(menu)
	parent.add_submenu_item(title, str(menu.name))
	menu.id_pressed.connect(_pressed)
	return menu

func _item(menu: PopupMenu, title: String, action: String, args: Dictionary = {}, enabled: bool = true, reason: String = "") -> void:
	menu.add_item(title, _commands.size())
	menu.set_item_disabled(menu.item_count - 1, not enabled)
	if not enabled: menu.set_item_tooltip(menu.item_count - 1, reason)
	_commands.append({"action": action, "args": args})

func _show(mouse: Vector2) -> void:
	editor._view._apply_editor_theme(popup)
	popup.reset_size()
	popup.position = Vector2i(mouse)
	popup.popup()

func valid(captured: Dictionary) -> bool:
	return captured.get("generation", -1) == generation

func _pressed(id: int) -> void:
	if id < 0 or id >= _commands.size(): return
	var command := _commands[id]
	var captured := context.duplicate(true)
	popup.hide()
	dispatch.call_deferred(command.action, captured, command.args)

func dispatch(action: String, captured: Dictionary, args: Dictionary = {}) -> void:
	if not valid(captured): return
	if action.begins_with("library_"):
		library.execute(action, captured.get("library", {}))
		return
	var objects: Array[Dictionary] = actions._typed(captured.get("targets", []))
	var hit: Dictionary = captured.get("hit", {})
	if action.begins_with("lod_"):
		# Inspector selection can change independently of document revision.
		actions.select(objects)
	match action:
		"new": editor._session.new_asset_dialog()
		"properties": actions.properties(hit)
		"frame": actions.frame(objects, hit.get("kind", ""))
		"frame_all":
			var all: Array[Dictionary] = []
			for index in editor._parts.size(): all.append({"kind": "mesh", "index": index})
			for index in editor._site_anchors_data.size(): all.append({"kind": "anchor", "index": index})
			for index in editor._site_surfaces_data.size(): all.append({"kind": "surface", "index": index})
			actions.frame(all)
		"rename": actions.rename(objects, func(): return valid(captured))
		"delete", "material", "delete_vertex", "insert_vertex": actions.edit(action, objects, args)
		"rotate": actions.begin_rotation(objects, args.get("angle", NAN))
		"duplicate":
			actions.begin_placement("duplicate", objects, {}, captured.get("ground"))
		"create": actions.begin_placement("create", [], args, captured.ground)
		"import": actions.import_at(captured.ground, func(): return valid(captured))
		"entrance_select": actions.properties({"kind": "anchor", "index": args.index})
		"entrance_reset":
			editor._on_reset_main_entrance_pressed()
			editor._session.capture_geometry("Reset entrance to frontage")
		"lod_add": editor._on_add_part_lod_requested()
		"lod_replace": editor._session.relink_selected_part(captured.lod)
		"lod_preview": editor._on_preview_lod_selected(args.tier)
		"lod_remove": editor._view._preview_panel._remove_last()
		"reference_reset": editor._preview.set_scale_reference_world_position(Vector3(0, 0.9, 0))
		"reference_hide": editor._view.scale_reference_button.button_pressed = false
		"reference_toggle": editor._view.scale_reference_button.button_pressed = not editor._view.scale_reference_button.button_pressed
		"ghost_clear": editor._on_clear_ghost_pressed()
		"lighting": editor._view.open_preview_popup()
