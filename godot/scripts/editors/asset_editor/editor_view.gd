# SPDX-License-Identifier: GPL-2.0-only

## Asset editor view: panel layout, control references and theme styling.
## Controller callbacks own authoring actions; this view owns no simulation or file I/O.
extends RefCounted

const TopMenu = preload("res://scripts/ui/top_menu.gd")
const EditorTheme = preload("res://scripts/ui/editor_theme.gd")
const SelectionRectOverlay = preload("res://scripts/editors/selection_rect_overlay.gd")
const PreviewPanel = preload("res://scripts/editors/asset_editor/preview_panel.gd")
const Spacing = preload("res://scripts/editors/asset_editor/editor_spacing.gd")
const PickingOverlay = preload("res://scripts/editors/asset_editor/picking_overlay.gd")

var _editor: Node3D
var _log_label: RichTextLabel
var _asset_tree: Tree
var _asset_search_edit: LineEdit
var _asset_count_lbl: Label
var _preview_panel: PreviewPanel
var _main_vsplit: VSplitContainer
var _left_split: HSplitContainer
var _right_split: HSplitContainer
var _preview_view_rect: Control
var _selection_rect_overlay: Control
var _theme_root: Control
var _pack_summary_lbl: Label
var _pack_set_btn: Button
var _pack_select_menu: PopupMenu
var _pack_create_window: Window
var _new_pack_id_edit: LineEdit
var _new_pack_name_edit: LineEdit
var _new_pack_author_edit: LineEdit
var _retarget_export_window: Window
var _retarget_export_message_lbl: Label
var _asset_id_edit: LineEdit
var _display_name_edit: LineEdit
var _width_spin: SpinBox
var _depth_spin: SpinBox
var _residents_spin: SpinBox
var _economy_profile_btn: OptionButton
var _economy_profile_status_lbl: Label
var _mesh_part_list: ItemList
var _frontage_lbl: Label  # shows current frontage forward vector
var _site_anchor_list: ItemList
var _site_anchor_name_edit: LineEdit
var _site_anchor_vehicle_class_btn: OptionButton
var _site_anchor_x_spin: SpinBox
var _site_anchor_y_spin: SpinBox
var _site_anchor_z_spin: SpinBox
var _site_anchor_yaw_spin: SpinBox
var _site_anchor_width_spin: SpinBox
var _site_anchor_length_spin: SpinBox
var _site_surface_list: ItemList
var _site_surface_name_edit: LineEdit
var _site_surface_material_btn: OptionButton
var _site_surface_y_spin: SpinBox
var _preview_scale_spin: SpinBox
var _part_x_spin: SpinBox
var _part_y_spin: SpinBox
var _part_z_spin: SpinBox
var _part_rotation_y_spin: SpinBox
var _dim_label: Label          # live "→ W × D × H m" display
var scale_reference_button: CheckButton
var _font_size_header:  int = 14   # section title labels ("Asset Browser", "Building Importer")
var _font_size_section: int = 12   # sub-section labels ("Pack", "Asset", "Building", etc.)
var _font_size_label:   int = 11   # spinbox labels and small info text
var _theme_mode: String = EditorTheme.MODE_DARK

const Panels = preload("res://scripts/editors/asset_editor/task_panels.gd")
const TASKS := ["overview", "model", "site", "gameplay", "validate"]
const TASK_NAMES := ["Overview", "Model", "Site", "Gameplay", "Validate & export"]

var fields: Dictionary = {}
var rows: Dictionary = {}
var tasks: Dictionary = {}
var task_picker: OptionButton
var library: Control
var diagnostics: Control
var preview_popup: AcceptDialog
var status: Label
var type_label: Label
var derived_workers: Label
var issues_box: VBoxContainer
var thumbnail: TextureRect
var part_properties: Control
var anchor_properties: Control
var surface_properties: Control
var site_picker: OptionButton
var site_sections: Array[Control] = []
var advanced_groups: Dictionary = {}
var undo_button: Button
var redo_button: Button
var catalog_refresh: Button
var export_summary: Label
var inspector: PanelContainer
var welcome: PanelContainer
var toolbar: HFlowContainer
var preview_bar: HFlowContainer
var inspector_button: Button
var comparison_button: Button
var frame_button: Button
var remove_part_button: Button
var remove_anchor_button: Button
var remove_surface_button: Button
var model_hint: Label
var selection_filter: OptionButton
var picking_overlay: Control
var hover_label: Label
var _document_open := false

func configure(editor: Node3D) -> void:
	_editor = editor

func _build_ui() -> void:
	var canvas := CanvasLayer.new()
	_editor.add_child(canvas)
	_theme_root = VBoxContainer.new()
	_theme_root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_theme_root.offset_top = TopMenu.BAR_HEIGHT
	canvas.add_child(_theme_root)
	if _editor._top_menu != null:
		_editor._top_menu._shell.resized.connect(_update_top_inset)
		_update_top_inset()
	toolbar = HFlowContainer.new()
	_theme_root.add_child(toolbar)
	button(toolbar, "Library", toggle_library)
	button(toolbar, "New asset…", _editor.menu_new_asset)
	button(toolbar, "Export asset…", _editor.menu_export_asset).tooltip_text = "Review the destination and validation issues, then export the asset to its game content pack."
	undo_button = button(toolbar, "Undo", _editor._session.undo)
	redo_button = button(toolbar, "Redo", _editor._session.redo)
	button(toolbar, "Diagnostics", toggle_diagnostics)
	inspector_button = button(toolbar, "Hide inspector", toggle_inspector)
	_main_vsplit = VSplitContainer.new()
	_main_vsplit.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_theme_root.add_child(_main_vsplit)
	_left_split = HSplitContainer.new()
	_main_vsplit.add_child(_left_split)
	_build_library(_left_split)
	_right_split = HSplitContainer.new()
	_right_split.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_left_split.add_child(_right_split)
	var center := VBoxContainer.new()
	center.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_right_split.add_child(center)
	preview_bar = HFlowContainer.new()
	center.add_child(preview_bar)
	button(preview_bar, "Lighting & quality…", open_preview_popup)
	frame_button = button(preview_bar, "Frame selection", _editor._frame_selected_part)
	scale_reference_button = CheckButton.new()
	scale_reference_button.flat = false
	scale_reference_button.text = "Scale reference"
	scale_reference_button.tooltip_text = "Show a 1.8 m scale reference. Click its figure or circular handle to select; left-drag moves it freely across the ground. Escape cancels. Preview only; never exported."
	scale_reference_button.toggled.connect(_editor._selection.set_scale_reference_visible)
	preview_bar.add_child(scale_reference_button)
	comparison_button = button(preview_bar, "Clear comparison", _editor._on_clear_ghost_pressed)
	comparison_button.hide()
	selection_filter = OptionButton.new()
	for title in ["Select: all objects", "Select: meshes only", "Select: anchors only", "Select: yards only"]:
		selection_filter.add_item(title)
	selection_filter.tooltip_text = "All objects is the default: click a mesh, anchor or yard to open its settings. These optional filters restrict selection only when explicitly chosen.\nClick selects; drag moves; right-click opens actions; R rotates. Alt+click cycles overlaps. Ctrl+click adds/removes; Shift+drag box-selects. Escape cancels an operation."
	selection_filter.item_selected.connect(_editor._selection.set_filter)
	preview_bar.add_child(selection_filter)
	_build_welcome(center)
	_preview_view_rect = Control.new()
	_preview_view_rect.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_preview_view_rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_preview_view_rect.clip_contents = true
	center.add_child(_preview_view_rect)
	picking_overlay = PickingOverlay.new()
	picking_overlay.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	picking_overlay.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_preview_view_rect.add_child(picking_overlay)
	hover_label = Label.new()
	hover_label.set_anchors_and_offsets_preset(Control.PRESET_TOP_WIDE)
	hover_label.offset_left = Spacing.CONTENT_PADDING
	hover_label.offset_right = -Spacing.CONTENT_PADDING
	hover_label.offset_top = Spacing.CONTROL_GAP
	hover_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	hover_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	hover_label.add_theme_color_override("font_outline_color", Color.BLACK)
	hover_label.add_theme_constant_override("outline_size", 4)
	_preview_view_rect.add_child(hover_label)
	_selection_rect_overlay = SelectionRectOverlay.new()
	_selection_rect_overlay.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_selection_rect_overlay.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_preview_view_rect.add_child(_selection_rect_overlay)
	_editor._cam_input.viewport_rect_control = _preview_view_rect
	_build_preview_popup(canvas)
	inspector = PanelContainer.new()
	inspector.custom_minimum_size.x = 340
	_right_split.add_child(inspector)
	var body := VBoxContainer.new()
	_add_panel_margin(inspector).add_child(body)
	task_picker = OptionButton.new()
	task_picker.name = "AuthoringTask"
	for title in TASK_NAMES:
		task_picker.add_item(title)
	task_picker.item_selected.connect(func(index): show_task(TASKS[index]))
	body.add_child(task_picker)
	for index in TASKS.size():
		var scroll := ScrollContainer.new()
		scroll.name = TASK_NAMES[index]
		scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
		scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
		body.add_child(scroll)
		var box := VBoxContainer.new()
		box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		_add_panel_margin(scroll).add_child(box)
		tasks[TASKS[index]] = box
	Panels.new().build(self)
	_build_diagnostics(_main_vsplit)
	library.visible = false
	diagnostics.visible = false
	show_task("model")
	_apply_editor_theme(_theme_root)
	set_document_open(false)
	_editor._layout.configure(_editor, self)

func _build_welcome(parent: Control) -> void:
	welcome = PanelContainer.new()
	welcome.size_flags_vertical = Control.SIZE_EXPAND_FILL
	parent.add_child(welcome)
	var center := CenterContainer.new()
	welcome.add_child(center)
	var box := VBoxContainer.new()
	box.custom_minimum_size.x = 320
	center.add_child(box)
	var title := Label.new()
	title.text = "Asset editor"
	title.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	title.add_theme_font_size_override("font_size", 24)
	box.add_child(title)
	button(box, "New asset…", _editor.menu_new_asset)
	button(box, "Browse asset library", toggle_library)

func set_document_open(active: bool) -> void:
	var changed := _document_open != active
	_document_open = active
	welcome.visible = not active
	toolbar.visible = active
	preview_bar.visible = active
	_preview_view_rect.visible = active
	inspector.visible = active and not _editor._layout.inspector_collapsed
	_editor._preview.visible = active
	_editor._cam_input.set_process(active)
	_editor._cam_input.set_process_input(active)
	_editor._top_menu.set_asset_document_open(active)
	_update_top_inset()
	if changed:
		library.hide()
		diagnostics.hide()
		_editor._layout.request_layout()
	update_context_actions()

func update_context_actions() -> void:
	frame_button.visible = _document_open and (_editor._has_selected_mesh_part() or _editor._selected_site_anchor_index >= 0 or _editor._selected_site_surface_index >= 0)
	remove_part_button.visible = _document_open and _editor._has_selected_mesh_part()
	remove_anchor_button.visible = _document_open and _editor._selected_site_anchor_index >= 0
	remove_surface_button.visible = _document_open and _editor._selected_site_surface_index >= 0
	_mesh_part_list.visible = not _editor._parts.is_empty()
	model_hint.visible = _editor._parts.is_empty()
	comparison_button.visible = _document_open and _editor._preview._ghost_has_mesh
	inspector_button.text = "Hide inspector" if inspector.visible else "Show inspector"

func toggle_library() -> void:
	library.visible = not library.visible
	_editor._layout.request_layout()

func toggle_diagnostics() -> void:
	diagnostics.visible = not diagnostics.visible
	_editor._layout.request_layout()

func toggle_inspector() -> void:
	_editor._layout.inspector_collapsed = not _editor._layout.inspector_collapsed
	inspector.visible = _document_open and not _editor._layout.inspector_collapsed
	update_context_actions()
	_editor._layout.request_layout()

func _build_library(parent: Node) -> void:
	library = PanelContainer.new()
	library.custom_minimum_size.x = 240
	parent.add_child(library)
	var body := VBoxContainer.new()
	_add_panel_margin(library).add_child(body)
	_add_label(body, "Asset library", _font_size_header)
	button(body, "Close library", toggle_library)
	_asset_search_edit = _add_line_edit(body, "Search assets", "")
	_asset_search_edit.text_changed.connect(_editor._on_asset_search_changed)
	_asset_count_lbl = Label.new()
	body.add_child(_asset_count_lbl)
	_asset_tree = Tree.new()
	_asset_tree.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_asset_tree.hide_root = true
	_asset_tree.item_activated.connect(_editor._on_asset_tree_activated)
	_asset_tree.gui_input.connect(_editor._on_asset_tree_gui_input)
	body.add_child(_asset_tree)

func _update_top_inset() -> void:
	_theme_root.offset_top = maxf(TopMenu.BAR_HEIGHT, _editor._top_menu._shell.size.y) + (Spacing.CONTROL_GAP if _document_open else 0)

func _build_preview_popup(parent: Node) -> void:
	preview_popup = AcceptDialog.new()
	preview_popup.title = "Preview lighting & quality"
	preview_popup.get_ok_button().text = "Close"
	parent.add_child(preview_popup)
	var scroll := ScrollContainer.new()
	scroll.custom_minimum_size = Vector2(360, 480)
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	preview_popup.add_child(scroll)
	_preview_panel = PreviewPanel.new()
	_preview_panel.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_preview_panel.hour_changed.connect(_editor._lighting.pin_hour_of_day)
	_preview_panel.emission_changed.connect(_editor._preview.set_preview_emission)
	_preview_panel.lod_selected.connect(_editor._on_preview_lod_selected)
	_preview_panel.quality_changed.connect(_editor._on_preview_quality_changed)
	_preview_panel.part_changed.connect(func(): _editor._sync_preview_lods(); _editor._session.capture_geometry("Edit LOD chain"))
	_preview_panel.add_lod_requested.connect(_editor._on_add_part_lod_requested)
	_preview_panel.replace_lod_requested.connect(_editor._session.relink_selected_part)
	scroll.add_child(_preview_panel)

func open_preview_popup() -> void:
	preview_popup.popup_centered(Vector2i(400, 560))

func _build_diagnostics(parent: Node) -> void:
	diagnostics = PanelContainer.new()
	diagnostics.custom_minimum_size.y = 120
	parent.add_child(diagnostics)
	var body := VBoxContainer.new()
	_add_panel_margin(diagnostics).add_child(body)
	button(body, "Copy diagnostic log", _editor._on_copy_log_pressed)
	_log_label = RichTextLabel.new()
	_log_label.custom_minimum_size.y = 50
	_log_label.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_log_label.bbcode_enabled = true
	_log_label.selection_enabled = true
	_log_label.scroll_following = true
	body.add_child(_log_label)

func show_task(id: String) -> void:
	if not tasks.has(id):
		return
	task_picker.select(TASKS.find(id))
	for key in tasks:
		tasks[key].get_parent().get_parent().visible = key == id
	_editor._selection.context_changed()

func show_site(index: int) -> void:
	site_picker.select(index)
	for i in site_sections.size():
		site_sections[i].visible = i == index
	_editor._selection.context_changed()

func reveal_field(section: String, field: String) -> void:
	show_task(section)
	if section == "site":
		show_site(1 if field == "anchors" else (2 if field == "site_surfaces" else 0))
	if fields.has(field):
		var control: Control = fields[field]
		if not rows[field].visible:
			show_task("validate")
			return
		var ancestor := control.get_parent()
		while ancestor != null:
			if ancestor is Control:
				ancestor.show()
			ancestor = ancestor.get_parent()
			if ancestor == tasks.get(section):
				break
		control.grab_focus()

func button(parent: Node, title: String, action: Callable) -> Button:
	var control := Button.new()
	control.text = title
	control.pressed.connect(action)
	parent.add_child(control)
	return control

func geometry_button(parent: Node, title: String, action: Callable) -> Button:
	return button(parent, title, func():
		action.call()
		_editor._session.capture_geometry(title)
	)

func field_row(parent: Node, key: String, title: String, control: Control) -> Control:
	var row := VBoxContainer.new()
	parent.add_child(row)
	_add_label(row, title, _font_size_label)
	control.name = key
	control.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	row.add_child(control)
	fields[key] = control
	rows[key] = row
	return control

func text_field(parent: Node, key: String, title: String) -> LineEdit:
	return field_row(parent, key, title, LineEdit.new())

func number_field(parent: Node, key: String, title: String, maximum: float = 9999, step: float = 1.0) -> SpinBox:
	var control := SpinBox.new()
	control.min_value = 0
	control.max_value = maximum
	control.allow_greater = true
	control.step = step
	return field_row(parent, key, title, control)

func choice_field(parent: Node, key: String, title: String, choices: Array = []) -> OptionButton:
	var control := OptionButton.new()
	control.fit_to_longest_item = false
	control.clip_text = true
	for choice in choices:
		control.add_item(str(choice).capitalize())
		control.set_item_metadata(control.item_count - 1, choice)
	return field_row(parent, key, title, control)

func advanced(parent: Node, key: String, title: String = "Advanced") -> VBoxContainer:
	var toggle := CheckButton.new()
	toggle.text = title
	parent.add_child(toggle)
	var box := VBoxContainer.new()
	box.visible = false
	parent.add_child(box)
	toggle.toggled.connect(func(value): box.visible = value)
	advanced_groups[key] = {"toggle": toggle, "body": box}
	return box

func _add_panel_margin(parent: Control) -> MarginContainer:
	var margin := MarginContainer.new()
	margin.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	margin.size_flags_vertical = Control.SIZE_EXPAND_FILL
	Spacing.pad_container(margin)
	parent.add_child(margin)
	return margin

func _add_label(parent: Control, text: String, size: int) -> void:
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	label.add_theme_font_size_override("font_size", size)
	parent.add_child(label)

func _add_line_edit(parent: Control, placeholder: String, default_val: String) -> LineEdit:
	var edit := LineEdit.new()
	edit.placeholder_text = placeholder
	edit.text = default_val
	edit.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	parent.add_child(edit)
	return edit

func _apply_editor_theme(root: Node) -> void:
	if root is AcceptDialog:
		_theme_dialog(root)
	EditorTheme.apply_to_tree(root, _theme_mode)
	Spacing.apply_to_tree(root)
	# This label overlays the 3D scene, not a themed panel background.
	if hover_label != null:
		hover_label.add_theme_color_override("font_color", Color.WHITE)
	if root != _theme_root:
		return
	# Unlike inspector switches, this toggle sits over the sky. Reuse the adjacent button's
	# opaque palette and padding for every state, including checked + hovered.
	for state in ["normal", "hover", "pressed", "hover_pressed", "focus", "disabled"]:
		scale_reference_button.add_theme_stylebox_override(state, frame_button.get_theme_stylebox("pressed" if state == "hover_pressed" else state))
	scale_reference_button.add_theme_color_override("font_hover_pressed_color", EditorTheme.color(_theme_mode, "text"))
	for extra in [_editor._menus.popup, _pack_select_menu, _pack_create_window, _retarget_export_window, preview_popup]:
		if is_instance_valid(extra):
			if extra is AcceptDialog:
				_theme_dialog(extra)
			EditorTheme.apply_to_tree(extra, _theme_mode)
			Spacing.apply_to_tree(extra)

func _theme_dialog(dialog: AcceptDialog) -> void:
	var dialog_theme := Theme.new()
	dialog_theme.set_stylebox("panel", "AcceptDialog", EditorTheme.style_box(_theme_mode, "panel", 4, 1))
	dialog.theme = dialog_theme
