# SPDX-License-Identifier: GPL-2.0-only

## Working preview controls, independent of the building metadata inspector.
## Lighting, quality and forced tiers are preview-only; creators supply ordered meshes.
extends VBoxContainer

const MeshPart := preload("res://scripts/editors/asset_editor/mesh_part.gd")
const Spacing := preload("res://scripts/editors/asset_editor/editor_spacing.gd")
signal hour_changed(hour: float)
signal emission_changed(mode: int, strength: float)
signal lod_selected(index: int)
signal quality_changed(index: int)
signal add_lod_requested
signal part_changed
signal frame_requested

var _part: MeshPart
var _lod_list: OptionButton
var _quality: OptionButton
var _bands: Label
var _metrics: Label
var _policy := AssetLodPolicy.new()
var _add: Button
var _remove: Button
var _emission: OptionButton
var _hour: SpinBox
var _strength: SpinBox
var _status: Label
var _selected_lod := -1
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
	_hour.value_changed.connect(func(value: float): hour_changed.emit(value))
	for preset in [["Day", 10.5], ["Dusk", 21.0], ["Night", 0.0]]:
		var button := Button.new()
		button.text = preset[0]
		button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		button.pressed.connect(func(): _hour.value = preset[1])
		presets.add_child(button)
	_section_gap()
	_label("Window emission (preview only)")
	_emission = OptionButton.new()
	for label in ["Authored default", "Force off", "Force on"]:
		_emission.add_item(label)
	_emission.item_selected.connect(func(_index: int): _emit_emission())
	_body.add_child(_emission)
	_strength = _spin("Reference intensity", 0.0, 10.0, 1.0, 0.1)
	_strength.value_changed.connect(func(_value: float): _emit_emission())
	_strength.editable = false
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
	_label("Selected part — LOD inspection")
	var frame := Button.new()
	frame.text = "Frame selected part"
	frame.pressed.connect(func(): frame_requested.emit())
	_body.add_child(frame)
	_lod_list = OptionButton.new()
	_lod_list.fit_to_longest_item = false
	_lod_list.clip_text = true
	_lod_list.item_selected.connect(_select_lod)
	_body.add_child(_lod_list)
	_section_gap()
	_metrics = _label("")
	_bands = _label("")
	_section_gap()
	var buttons := HFlowContainer.new()
	buttons.add_theme_constant_override("h_separation", Spacing.CONTROL_GAP)
	buttons.add_theme_constant_override("v_separation", Spacing.CONTROL_GAP)
	_body.add_child(buttons)
	_add = Button.new()
	_add.text = "Add LOD…"
	_add.pressed.connect(func(): add_lod_requested.emit())
	buttons.add_child(_add)
	_remove = Button.new()
	_remove.text = "Remove last LOD"
	_remove.pressed.connect(_remove_last)
	buttons.add_child(_remove)
	_section_gap()
	_status = _label("Select a mesh part to inspect its LODs.")
	_label("Calibration defaults, not gameplay settings. Saved metre bands do not control this preview.")
	set_part(null)

## LOD authoring belongs in Model; lighting and global quality remain viewport controls.
func move_lod_controls(parent: Control) -> void:
	_lod_controls.reparent(parent)
	_lod_controls.show()
	get_node("PreviewTabs").tabs_visible = false

func set_part(part: MeshPart, selected_lod: int = -1) -> void:
	_part = part
	_selected_lod = selected_lod
	_refresh()

func show_lod_state(active: int, pixels: float, triangles: int, error: String = "") -> void:
	var size_text := "near-plane intersection" if is_inf(pixels) else "%.1f render px" % pixels
	_metrics.text = "%s → LOD%d · %d triangles\nLOD0 bounds: %s" % [
		"Automatic" if _selected_lod < 0 else "Forced", active, triangles, size_text]
	_status.text = error if not error.is_empty() else "Placement, source materials and camera unchanged."

func set_quality(index: int) -> void:
	_quality.select(index)
	_update_bands()
	quality_changed.emit(index)

## Set preview state through the same controls/signals used by interactive inspection.
func set_lighting(hour: float, emission_mode: int, strength: float = 1.0) -> void:
	_hour.value = hour
	_emission.select(emission_mode)
	_strength.value = strength
	_emit_emission()

func _refresh() -> void:
	_lod_list.clear()
	_lod_list.add_item("Automatic (engine policy)")
	if _part != null:
		for index in _part.lods.size():
			var lod := _part.lods[index]
			_lod_list.add_item("Force LOD%d — %s" % [index, lod.file.get_file()])
	_selected_lod = clampi(_selected_lod, -1, _lod_list.item_count - 2)
	var has_part := _part != null and not _part.lods.is_empty()
	_add.disabled = not has_part
	_remove.disabled = not has_part or _part.lods.size() <= 1
	_lod_list.set_item_disabled(0, not has_part)
	if has_part:
		_lod_list.select(_selected_lod + 1)
		var error := _part.validation_error()
		_status.text = error
	else:
		_status.text = "Select a mesh part to inspect its LODs."
		_metrics.text = ""
	_update_bands()

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

func _emit_emission() -> void:
	_strength.editable = _emission.selected == 2
	emission_changed.emit(_emission.selected, _strength.value)

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
