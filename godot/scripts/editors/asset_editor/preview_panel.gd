# SPDX-License-Identifier: GPL-2.0-only

## Working preview controls, independent of the building metadata inspector.
## Lighting, quality and temporary tier inspection are preview-only; creators supply ordered meshes.
extends VBoxContainer

const MeshPart := preload("res://scripts/editors/asset_editor/mesh_part.gd")
const Spacing := preload("res://scripts/editors/asset_editor/editor_spacing.gd")
const PreviewMaterials := preload("res://scripts/editors/asset_editor/preview_materials.gd")
const DayCycleConfig := preload("res://scripts/core/day_cycle.gd")
const AUTOMATIC_EMISSION := -1
signal hour_changed(hour: float)
signal emission_changed(mode: int, strength: float)
signal lod_selected(index: int)
signal quality_changed(index: int)
signal add_lod_requested
signal replace_lod_requested(index: int)
signal part_changed

var _part: MeshPart
var _lod_list: ItemList
var _quality: OptionButton
var _bands: Label
var _metrics: Label
var _projected_size: Label
var _policy := AssetLodPolicy.new()
var _add: Button
var _replace: Button
var _remove: Button
var _emission: OptionButton
var _hour: SpinBox
var _strength: SpinBox
var _status: Label
# Forced preview choice and actually rendered tier are distinct when Automatic is enabled.
var _selected_lod := -1
var _active_lod := -1
var _body: VBoxContainer
var _lod_controls: MarginContainer

func _ready() -> void:
	var tabs := TabContainer.new()
	tabs.name = "PreviewTabs"
	tabs.use_hidden_tabs_for_min_size = false
	tabs.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	add_child(tabs)
	_body = _tab_body(tabs, "Lighting")
	_label("Preview lighting")
	var presets := HBoxContainer.new()
	presets.add_theme_constant_override("separation", Spacing.CONTROL_GAP)
	_body.add_child(presets)
	_hour = _spin("Hour of day", 0.0, 24.0, 10.5, 0.1)
	_hour.value_changed.connect(_on_hour_changed)
	for preset in [["Day", 10.5], ["Dusk", 21.0], ["Night", 0.0]]:
		var button := Button.new()
		button.name = preset[0]
		button.text = preset[0]
		button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		button.pressed.connect(func(): _hour.value = preset[1])
		presets.add_child(button)
	_section_gap()
	_label("Window emission (preview only)")
	_emission = OptionButton.new()
	# Automatic precedes the concrete PreviewMaterials.Mode values.
	for label in ["Automatic (time of day)", "Authored default", "Force off", "Force on"]:
		_emission.add_item(label)
	_emission.item_selected.connect(func(_index: int): _emit_emission())
	_body.add_child(_emission)
	_strength = _spin("Reference intensity", 0.0, 10.0, 1.0, 0.1)
	_strength.value_changed.connect(func(_value: float): _emit_emission())
	_section_gap()
	_label("Emission previews luminous surfaces, not ground light spill.")
	_label("Preview quality (all parts)")
	_quality = OptionButton.new()
	for label in ["Performance", "Balanced", "Quality"]:
		_quality.add_item(label)
	_quality.select(1)
	_quality.item_selected.connect(func(index: int):
		_update_bands()
		quality_changed.emit(index)
	)
	_body.add_child(_quality)
	_body = _tab_body(tabs, "LODs")
	_lod_controls = _body.get_parent()
	_label("Level of detail (LODs)")
	var buttons := HFlowContainer.new()
	buttons.add_theme_constant_override("h_separation", Spacing.CONTROL_GAP)
	buttons.add_theme_constant_override("v_separation", Spacing.CONTROL_GAP)
	_body.add_child(buttons)
	_add = Button.new()
	_add.text = "Add LOD…"
	_add.tooltip_text = "Add the next lower-detail mesh to this part, not a separate part. Preview updates immediately."
	_add.pressed.connect(func(): add_lod_requested.emit())
	buttons.add_child(_add)
	_replace = Button.new()
	_replace.tooltip_text = "Replace or relink the selected LOD file. In Automatic, follows the visible tier. Placement and other levels are preserved."
	_replace.pressed.connect(func(): replace_lod_requested.emit(maxi(0, _inspection_lod())))
	buttons.add_child(_replace)
	_remove = Button.new()
	_remove.text = "Remove last LOD"
	_remove.pressed.connect(_remove_last)
	buttons.add_child(_remove)
	_lod_list = ItemList.new()
	_lod_list.allow_reselect = true
	_lod_list.tooltip_text = "Click a LOD to preview it immediately. Move or zoom the camera to resume Automatic; the highlight follows the visible tier. No save or export needed."
	_lod_list.item_selected.connect(_select_lod)
	_body.add_child(_lod_list)
	_metrics = _label("")
	_status = _label("Select a mesh part to inspect its LODs.")
	var details_toggle := CheckButton.new()
	details_toggle.name = "LODDetails"
	details_toggle.text = "LOD details"
	_body.add_child(details_toggle)
	var details := VBoxContainer.new()
	details.hide()
	_body.add_child(details)
	details_toggle.toggled.connect(func(active): details.visible = active)
	_body = details
	_projected_size = _label("")
	_projected_size.tooltip_text = "Largest projected dimension of the original LOD0 bounds at the 3D render resolution. This stable reference drives automatic LOD selection; it is not the active mesh."
	_bands = _label("")
	_label("Calibration defaults, not gameplay settings. Saved metre bands do not control this preview.")
	set_part(null)
	_emit_emission()

## LOD authoring belongs in Model; lighting and global quality remain viewport controls.
func move_lod_controls(parent: Control) -> void:
	_lod_controls.reparent(parent)
	_lod_controls.show()
	get_node("PreviewTabs").tabs_visible = false

func set_part(part: MeshPart, selected_lod: int = -1, active_lod: int = -1) -> void:
	_part = part
	_selected_lod = selected_lod
	_active_lod = active_lod
	_refresh()

func show_lod_state(active: int, pixels: float, triangles: int, selected_lod: int, error: String = "") -> void:
	_active_lod = active
	_selected_lod = selected_lod
	_sync_lod_selection()
	var size_text := "near-plane intersection" if is_inf(pixels) else "%.1f render px" % pixels
	_projected_size.text = "Projected asset size: %s" % size_text
	_metrics.text = "%s → LOD%d · %d triangles" % [
		"Automatic" if _selected_lod < 0 else "Preview", active, triangles]
	if _selected_lod >= 0:
		_metrics.text += "\nMove camera to resume Automatic."
	_status.text = error
	_status.visible = not error.is_empty()

func set_quality(index: int) -> void:
	_quality.select(index)
	_update_bands()
	quality_changed.emit(index)

## Set preview state through the same controls/signals used by interactive inspection.
func set_lighting(hour: float, emission_mode: int = AUTOMATIC_EMISSION, strength: float = 1.0) -> void:
	_hour.set_value_no_signal(hour)
	_emission.select(emission_mode + 1)
	_strength.set_value_no_signal(strength)
	_on_hour_changed(_hour.value)

func _refresh() -> void:
	_lod_list.clear()
	_lod_list.add_item("Automatic — zoom to switch")
	if _part != null:
		for index in _part.lods.size():
			var lod := _part.lods[index]
			_lod_list.add_item("LOD%d — %s" % [index, lod.file.get_file()])
			_lod_list.set_item_tooltip(index + 1, lod.source_path)
	# Fit short chains without empty space; long chains scroll after five visible rows.
	var row_height := _lod_list.get_theme_font("font").get_height(_lod_list.get_theme_font_size("font_size")) + _lod_list.get_theme_constant("v_separation")
	_lod_list.custom_minimum_size.y = _lod_list.get_theme_stylebox("panel").get_minimum_size().y + row_height * mini(_lod_list.item_count, 5)
	_selected_lod = clampi(_selected_lod, -1, _lod_list.item_count - 2)
	var has_part := _part != null and not _part.lods.is_empty()
	_add.disabled = not has_part
	_add.text = "Add LOD%d…" % _part.lods.size() if has_part else "Add LOD…"
	_replace.disabled = _part == null
	_remove.disabled = not has_part or _part.lods.size() <= 1
	_remove.visible = not _remove.disabled
	_remove.text = "Remove LOD%d" % (_part.lods.size() - 1) if has_part and _part.lods.size() > 1 else "Remove last LOD"
	_lod_list.set_item_disabled(0, not has_part)
	if has_part:
		var error := _part.validation_error()
		_status.text = error
	else:
		_status.text = "Select a mesh part to inspect its LODs."
		_metrics.text = ""
		_projected_size.text = ""
	_status.visible = not _status.text.is_empty()
	_sync_lod_selection()
	_update_bands()

func _inspection_lod() -> int:
	# Keep an explicitly forced, failed tier selectable for source repair.
	return _selected_lod if _selected_lod >= 0 else _active_lod

func _sync_lod_selection() -> void:
	var automatic_text := "Automatic (on) — zoom to switch" if _selected_lod < 0 else "Automatic — zoom to switch"
	if _lod_list.get_item_text(0) != automatic_text:
		_lod_list.set_item_text(0, automatic_text)
	var tier := _inspection_lod()
	var row := clampi(tier + 1, 0, _lod_list.item_count - 1)
	if _lod_list.is_selected(row):
		return
	_replace.text = "Replace LOD%d…" % maxi(0, tier)
	if not _lod_list.is_item_disabled(row):
		# Programmatic highlighting must not emit a forced-mode change or rebuild the list.
		_lod_list.select(row)
		_lod_list.ensure_current_is_visible()

func _select_lod(index: int) -> void:
	_selected_lod = index - 1
	_refresh()
	lod_selected.emit(_selected_lod)

func _update_bands() -> void:
	_bands.text = ""
	if _part == null:
		return
	for tier in range(_part.lods.size() - 1):
		_bands.text += "LOD%d → %d below %.1f px\n" % [tier, tier + 1, _policy.switch_pixels(tier, _quality.selected)]
	_bands.text += "Stable transitions; culling is separate."

func _remove_last() -> void:
	if _part == null:
		return
	_part.remove_last_lod()
	_refresh()
	part_changed.emit()
	lod_selected.emit(_selected_lod)

func _on_hour_changed(hour: float) -> void:
	hour_changed.emit(hour)
	_emit_emission()

func _emit_emission() -> void:
	var mode := _emission.selected - 1
	_strength.editable = mode == AUTOMATIC_EMISSION or mode == PreviewMaterials.Mode.ON
	if mode == AUTOMATIC_EMISSION:
		# Follow the same solar arc as scene lighting, including the midnight wrap.
		var sun_elevation := DayCycleConfig.solar_position_deg(_hour.value / 24.0).x
		mode = PreviewMaterials.Mode.ON if sun_elevation <= 0.0 else PreviewMaterials.Mode.OFF
	emission_changed.emit(mode, _strength.value)

func _label(text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	label.add_theme_constant_override("line_spacing", Spacing.LINE_SPACING)
	_body.add_child(label)
	return label

func _tab_body(tabs: TabContainer, title: String) -> VBoxContainer:
	var margin := MarginContainer.new()
	margin.name = title
	Spacing.pad_container(margin)
	tabs.add_child(margin)
	var body := VBoxContainer.new()
	body.name = "Content"
	body.add_theme_constant_override("separation", Spacing.CONTROL_GAP)
	margin.add_child(body)
	return body

func _section_gap() -> void:
	var spacer := Control.new()
	spacer.custom_minimum_size.y = 4
	spacer.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_body.add_child(spacer)

func _spin(label: String, minimum: float, maximum: float, value: float, step: float) -> SpinBox:
	_label(label)
	var spin := SpinBox.new()
	spin.min_value = minimum
	spin.max_value = maximum
	spin.step = step
	spin.value = value
	_body.add_child(spin)
	return spin
