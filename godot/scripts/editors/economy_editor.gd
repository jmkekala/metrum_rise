# SPDX-License-Identifier: GPL-2.0-only

## Economy editor shell — launched via `--economy-editor` command-line argument.
## Loads the authoritative `economy/` TOML pack, visualizes authored scenario
## graphs, edits selected definitions in memory, and round-trips validation /
## export / sandbox playback through SimulationNode.
## Calls: sim.is_economy_editor_mode(), sim.load_economy_project(dir),
##        sim.export_economy_project(project_json, dir),
##        sim.run_economy_sandbox(project_json, scenario_id)
extends Node

const TopMenu = preload("res://scripts/ui/top_menu.gd")

const PANEL_LEFT_W := 300
const PANEL_RIGHT_W := 330
const PANEL_BOTTOM_H := 220

@onready var sim: SimulationNode = $SimulationNode

var _economy_dir := ""
var _project: Dictionary = {}
var _validation: Array = []
var _sandbox_result: Dictionary = {}
var _sandbox_error := ""
var _scenario_ids: Array[String] = []
var _current_scenario_id := ""
var _selected_kind := ""
var _selected_id := ""
var _selected_node_id := ""

var _status_label: Label
var _scenario_picker: OptionButton
var _browser_tree: Tree
var _graph_canvas: Control
var _inspector_body: VBoxContainer
var _diagnostics_label: TextEdit

func _ready() -> void:
	if not sim.is_economy_editor_mode():
		push_error("EconomyEditor scene loaded without --economy-editor flag")

	_attach_top_menu()
	_economy_dir = ProjectSettings.globalize_path("res://../economy")
	_build_ui()
	_load_project()

func _build_ui() -> void:
	var canvas := CanvasLayer.new()
	add_child(canvas)

	var root := VBoxContainer.new()
	root.set_anchors_preset(Control.PRESET_FULL_RECT)
	root.offset_top = TopMenu.BAR_HEIGHT
	root.add_theme_constant_override("separation", 0)
	canvas.add_child(root)

	_build_toolbar(root)

	var row := HBoxContainer.new()
	row.size_flags_vertical = Control.SIZE_EXPAND_FILL
	row.add_theme_constant_override("separation", 0)
	root.add_child(row)

	_build_left_panel(row)
	_build_center_panel(row)
	_build_right_panel(row)
	_build_bottom_panel(root)

func _attach_top_menu() -> void:
	if has_node("TopMenu"):
		return
	var top_menu := TopMenu.new()
	top_menu.name = "TopMenu"
	top_menu.scene_kind = TopMenu.SCENE_ECONOMY_EDITOR
	add_child(top_menu)

func menu_save() -> void:
	_export_project()

func menu_return_to_game() -> void:
	_spawn_project_instance([])
	get_tree().quit()

func menu_reload_project() -> void:
	_load_project()

func menu_run_sandbox() -> void:
	_run_sandbox()

func _spawn_project_instance(arguments: PackedStringArray) -> void:
	var launch_args := PackedStringArray()
	if not arguments.is_empty():
		launch_args.append("--")
		launch_args.append_array(arguments)
	var pid := OS.create_instance(launch_args)
	if pid == -1:
		push_error("Failed to launch a new project instance.")

func _build_toolbar(parent: Control) -> void:
	var panel := PanelContainer.new()
	parent.add_child(panel)

	var bar := HBoxContainer.new()
	bar.add_theme_constant_override("separation", 8)
	panel.add_child(bar)

	var title := Label.new()
	title.text = "Economy Editor"
	title.custom_minimum_size.x = 170
	bar.add_child(title)

	var path_label := Label.new()
	path_label.text = "Source: %s" % _economy_dir
	path_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	bar.add_child(path_label)

	_scenario_picker = OptionButton.new()
	_scenario_picker.custom_minimum_size.x = 220
	_scenario_picker.item_selected.connect(_on_scenario_selected)
	bar.add_child(_scenario_picker)

	var reload_btn := Button.new()
	reload_btn.text = "Reload"
	reload_btn.pressed.connect(_load_project)
	bar.add_child(reload_btn)

	var export_btn := Button.new()
	export_btn.text = "Export"
	export_btn.pressed.connect(_export_project)
	bar.add_child(export_btn)

	var run_btn := Button.new()
	run_btn.text = "Run Sandbox"
	run_btn.pressed.connect(_run_sandbox)
	bar.add_child(run_btn)

	_status_label = Label.new()
	_status_label.custom_minimum_size.x = 280
	_status_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	bar.add_child(_status_label)

func _build_left_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.x = PANEL_LEFT_W
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
	parent.add_child(panel)

	var vbox := VBoxContainer.new()
	vbox.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panel.add_child(vbox)

	var title := Label.new()
	title.text = "Definitions"
	vbox.add_child(title)

	_browser_tree = Tree.new()
	_browser_tree.columns = 1
	_browser_tree.hide_root = true
	_browser_tree.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_browser_tree.item_selected.connect(_on_browser_item_selected)
	vbox.add_child(_browser_tree)

func _build_center_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
	parent.add_child(panel)

	var vbox := VBoxContainer.new()
	vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	vbox.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panel.add_child(vbox)

	var title := Label.new()
	title.text = "Scenario Graph"
	vbox.add_child(title)

	var graph_script := load("res://scripts/editors/economy_graph_canvas.gd")
	_graph_canvas = Control.new()
	_graph_canvas.set_script(graph_script)
	_graph_canvas.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_graph_canvas.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_graph_canvas.connect("node_selected", _on_graph_node_selected)
	_graph_canvas.connect("node_moved", _on_graph_node_moved)
	vbox.add_child(_graph_canvas)

func _build_right_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.x = PANEL_RIGHT_W
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
	parent.add_child(panel)

	var scroll := ScrollContainer.new()
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panel.add_child(scroll)

	_inspector_body = VBoxContainer.new()
	_inspector_body.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.add_child(_inspector_body)

func _build_bottom_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.y = PANEL_BOTTOM_H
	parent.add_child(panel)

	_diagnostics_label = TextEdit.new()
	_diagnostics_label.editable = false
	_diagnostics_label.selecting_enabled = true
	_diagnostics_label.context_menu_enabled = true
	_diagnostics_label.wrap_mode = TextEdit.LINE_WRAPPING_BOUNDARY
	_diagnostics_label.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panel.add_child(_diagnostics_label)

func _load_project() -> void:
	var payload := _parse_json(sim.load_economy_project(_economy_dir))
	if not payload.get("ok", false):
		_set_status("Load failed: %s" % str(payload.get("error", "unknown error")), true)
		return

	_project = payload.get("project", {})
	_validation = payload.get("validation", [])
	_sandbox_result = {}
	_sandbox_error = ""
	_rebuild_browser()
	_rebuild_scenario_picker()
	if _current_scenario_id.is_empty() and not _scenario_ids.is_empty():
		_current_scenario_id = _scenario_ids[0]
	if _selected_kind.is_empty():
		if not _current_scenario_id.is_empty():
			_select_item("scenario", _current_scenario_id, "")
	else:
		_refresh_inspector()
	_refresh_graph()
	_refresh_diagnostics()
	_set_status("Loaded economy project from %s" % _economy_dir)

func _export_project() -> void:
	if _project.is_empty():
		_set_status("Nothing to export yet.", true)
		return
	var payload := _parse_json(sim.export_economy_project(JSON.stringify(_project), _economy_dir))
	_validation = payload.get("validation", [])
	_refresh_diagnostics()
	if payload.get("ok", false):
		_set_status("Exported economy pack and rebuilt economy.index.bin")
		_load_project()
	else:
		_set_status("Export failed: %s" % str(payload.get("error", "unknown error")), true)

func _run_sandbox() -> void:
	if _project.is_empty() or _current_scenario_id.is_empty():
		_set_status("Select a scenario before sandbox playback.", true)
		return
	var payload := _parse_json(sim.run_economy_sandbox(JSON.stringify(_project), _current_scenario_id))
	_validation = payload.get("validation", [])
	if payload.get("ok", false):
		_sandbox_result = payload.get("result", {})
		_sandbox_error = ""
		_set_status("Sandbox playback finished for %s" % _current_scenario_id)
	else:
		_sandbox_error = str(payload.get("error", "unknown error"))
		_set_status("Sandbox failed: %s" % str(payload.get("error", "unknown error")), true)
	_refresh_diagnostics()

func _rebuild_browser() -> void:
	_browser_tree.clear()
	var root := _browser_tree.create_item()
	_add_browser_section(root, "Profiles", _project.get("profiles", []), "profile")
	_add_browser_section(root, "Controllers", _project.get("controllers", []), "controller")
	_add_browser_section(root, "Scenarios", _project.get("scenarios", []), "scenario")

func _add_browser_section(root: TreeItem, title: String, items: Array, kind: String) -> void:
	var section := _browser_tree.create_item(root)
	section.set_text(0, title)
	section.set_selectable(0, false)
	for item_data in items:
		var item := _browser_tree.create_item(section)
		var display_name := str(item_data.get("display_name", item_data.get("id", "")))
		item.set_text(0, "%s (%s)" % [display_name, str(item_data.get("id", ""))])
		item.set_metadata(0, {"kind": kind, "id": str(item_data.get("id", ""))})
		if kind == _selected_kind and str(item_data.get("id", "")) == _selected_id:
			_browser_tree.set_selected(item, 0)

func _rebuild_scenario_picker() -> void:
	_scenario_picker.clear()
	_scenario_ids.clear()
	for scenario in _project.get("scenarios", []):
		_scenario_picker.add_item(str(scenario.get("display_name", scenario.get("id", ""))))
		_scenario_ids.append(str(scenario.get("id", "")))
	var selected_idx := 0
	for idx in range(_scenario_ids.size()):
		if _scenario_ids[idx] == _current_scenario_id:
			selected_idx = idx
			break
	if not _scenario_ids.is_empty():
		_current_scenario_id = _scenario_ids[selected_idx]
		_scenario_picker.select(selected_idx)

func _refresh_graph() -> void:
	if _graph_canvas == null:
		return
	var scenario := _get_selected_scenario()
	if scenario.is_empty():
		_graph_canvas.call("set_graph", [], [], "")
		return
	var nodes: Array = []
	for node in scenario.get("nodes", []):
		var ref_kind := str(node.get("ref_kind", ""))
		var ref_id := str(node.get("ref_id", ""))
		var title := ref_id
		var subtitle := ref_id
		var tooltip := ""
		if ref_kind == "profile":
			var profile := _find_entry("profiles", ref_id)
			title = str(profile.get("display_name", ref_id))
			tooltip = _ports_summary(profile)
		elif ref_kind == "controller":
			var controller := _find_entry("controllers", ref_id)
			title = str(controller.get("display_name", ref_id))
			tooltip = str(controller.get("description", ""))
		nodes.append({
			"id": str(node.get("id", "")),
			"kind": ref_kind,
			"title": title,
			"subtitle": subtitle,
			"position": node.get("position", [40.0, 40.0]),
			"tooltip": tooltip,
		})

	var edges: Array = []
	for edge in scenario.get("edges", []):
		edges.append({
			"from": str(edge.get("from", "")),
			"to": str(edge.get("to", "")),
			"resource": str(edge.get("resource", "")),
			"kind": "resource",
		})
	for link in scenario.get("controller_links", []):
		edges.append({
			"from": str(link.get("controller_node_id", "")),
			"to": str(link.get("target_node_id", "")),
			"kind": "controller",
		})

	var highlighted_node_id := _selected_node_id
	if highlighted_node_id.is_empty() and (_selected_kind == "profile" or _selected_kind == "controller"):
		highlighted_node_id = _find_graph_node_for_ref(_selected_kind, _selected_id)
	_graph_canvas.call("set_graph", nodes, edges, highlighted_node_id)

func _refresh_inspector() -> void:
	for child in _inspector_body.get_children():
		child.queue_free()

	var title := Label.new()
	title.text = "Inspector"
	_inspector_body.add_child(title)

	if _selected_kind.is_empty():
		var hint := Label.new()
		hint.text = "Select a profile, controller, scenario, or graph node."
		hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		_inspector_body.add_child(hint)
		return

	match _selected_kind:
		"profile":
			_build_profile_inspector()
		"controller":
			_build_controller_inspector()
		"scenario":
			_build_scenario_inspector()

func _build_profile_inspector() -> void:
	var profile := _find_entry("profiles", _selected_id)
	_add_line_field("Profile ID", _selected_id, false)
	_add_line_field("Display Name", str(profile.get("display_name", "")), true, _update_selected_entry.bind("display_name"))
	_add_line_field("Kind", str(profile.get("kind", "")), true, _update_selected_entry.bind("kind"))
	_add_line_field("Description", str(profile.get("description", "")), true, _update_selected_entry.bind("description"))
	_add_spin_field("Worker Capacity", float(profile.get("worker_capacity", 0)), 0, 9999, 1, _update_selected_entry.bind("worker_capacity"))
	_add_spin_field("Base Rate / Day", float(profile.get("base_rate_units_per_day", 0.0)), 0, 10000, 1, _update_selected_entry.bind("base_rate_units_per_day"))
	_add_spin_field("Unit Price", float(profile.get("unit_price_currency", 0.0)), 0, 1000, 0.1, _update_selected_entry.bind("unit_price_currency"))
	_add_spin_field("Wage Min", float(profile.get("wage_min_currency_per_day", 0.0)), 0, 1000, 1, _update_selected_entry.bind("wage_min_currency_per_day"))
	_add_spin_field("Wage Max", float(profile.get("wage_max_currency_per_day", 0.0)), 0, 1000, 1, _update_selected_entry.bind("wage_max_currency_per_day"))
	_add_spin_field("Inventory Target (days)", float(profile.get("stock_target_days", 0.0)), 0, 30, 0.1, _update_selected_entry.bind("stock_target_days"))
	_add_spin_field("Reorder Threshold", float(profile.get("reorder_threshold_days", 0.0)), 0, 30, 0.1, _update_selected_entry.bind("reorder_threshold_days"))
	_add_spin_field("Critical Threshold", float(profile.get("critical_threshold_days", 0.0)), 0, 30, 0.1, _update_selected_entry.bind("critical_threshold_days"))
	_add_spin_field("Min Shipment Units", float(profile.get("min_shipment_units", 0.0)), 0, 10000, 1, _update_selected_entry.bind("min_shipment_units"))
	_add_spin_field("Consumption / Resident / Day", float(profile.get("consumption_rate_per_resident", 0.0)), 0, 10, 0.1, _update_selected_entry.bind("consumption_rate_per_resident"))
	
	var inputs: Array = profile.get("inputs", [])
	for i in range(inputs.size()):
		var res_name = str(inputs[i].get("resource", ""))
		var amt = float(inputs[i].get("units_per_day", 0.0))
		_add_spin_field("Input (%s)" % res_name, amt, 0, 100000, 1, _update_port_value.bind("inputs", i))
		
	var outputs: Array = profile.get("outputs", [])
	for i in range(outputs.size()):
		var res_name = str(outputs[i].get("resource", ""))
		var amt = float(outputs[i].get("units_per_day", 0.0))
		_add_spin_field("Output (%s)" % res_name, amt, 0, 100000, 1, _update_port_value.bind("outputs", i))
		
	_add_selected_node_position_fields()

func _build_controller_inspector() -> void:
	var controller := _find_entry("controllers", _selected_id)
	_add_line_field("Controller ID", _selected_id, false)
	_add_line_field("Display Name", str(controller.get("display_name", "")), true, _update_selected_entry.bind("display_name"))
	_add_line_field("Kind", str(controller.get("kind", "")), true, _update_selected_entry.bind("kind"))
	_add_line_field("Description", str(controller.get("description", "")), true, _update_selected_entry.bind("description"))
	_add_spin_field("Default Weight", float(controller.get("default_weight", 1.0)), 0, 1, 0.05, _update_selected_entry.bind("default_weight"))
	_add_spin_field("Min Multiplier", float(controller.get("min_multiplier", 1.0)), 0, 10, 0.05, _update_selected_entry.bind("min_multiplier"))
	_add_spin_field("Max Multiplier", float(controller.get("max_multiplier", 1.0)), 0, 10, 0.05, _update_selected_entry.bind("max_multiplier"))
	_add_selected_node_position_fields()

func _build_scenario_inspector() -> void:
	var scenario := _find_entry("scenarios", _selected_id)
	_add_line_field("Scenario ID", _selected_id, false)
	_add_line_field("Display Name", str(scenario.get("display_name", "")), true, _update_selected_entry.bind("display_name"))
	_add_line_field("Description", str(scenario.get("description", "")), true, _update_selected_entry.bind("description"))
	_add_spin_field("Duration (days)", float(scenario.get("duration_days", 30)), 1, 365, 1, _update_selected_entry.bind("duration_days"))
	_add_spin_field("Household Count", float(scenario.get("household_count", 0)), 0, 1000000, 1, _update_selected_entry.bind("household_count"))
	_add_spin_field("Average Household Size", float(scenario.get("average_household_size", 1.0)), 1, 10, 0.1, _update_selected_entry.bind("average_household_size"))
	_add_spin_field("Starting Supplies (days)", float(scenario.get("starting_household_stock_days", 0.0)), 0, 30, 0.1, _update_selected_entry.bind("starting_household_stock_days"))
	_add_spin_field("Target Supplies (days)", float(scenario.get("replenishment_target_days", 0.0)), 0, 30, 0.1, _update_selected_entry.bind("replenishment_target_days"))
	_add_spin_field("Trigger Supplies (days)", float(scenario.get("replenishment_trigger_days", 0.0)), 0, 30, 0.1, _update_selected_entry.bind("replenishment_trigger_days"))
	_add_spin_field("Pickup Cadence (hours)", float(scenario.get("pickup_cadence_hours", 0.0)), 0, 48, 0.5, _update_selected_entry.bind("pickup_cadence_hours"))
	_add_read_only_block("Graph", "Nodes: %d\nEdges: %d\nController links: %d" % [
		scenario.get("nodes", []).size(),
		scenario.get("edges", []).size(),
		scenario.get("controller_links", []).size(),
	])

func _add_selected_node_position_fields() -> void:
	if _selected_node_id.is_empty():
		return
	var node := _find_node_in_current_scenario(_selected_node_id)
	if node.is_empty():
		return
	var position: Array = node.get("position", [0.0, 0.0])
	_add_spin_field("Graph X", float(position[0]), 0, 4000, 1, _update_selected_node_position.bind(0))
	_add_spin_field("Graph Y", float(position[1]), 0, 4000, 1, _update_selected_node_position.bind(1))

func _add_line_field(label_text: String, value: String, editable: bool, callback: Callable = Callable()) -> void:
	var label := Label.new()
	label.text = label_text
	_inspector_body.add_child(label)
	var field := LineEdit.new()
	field.text = value
	field.editable = editable
	if editable and callback.is_valid():
		# Only commit on submit/focus-exit, never on text_changed.
		# text_changed rebuilds the inspector and steals focus every keystroke.
		field.text_submitted.connect(func(new_text):
			callback.call(new_text)
			_commit())
		field.focus_exited.connect(func():
			callback.call(field.text)
			_commit())
	_inspector_body.add_child(field)

func _add_spin_field(label_text: String, value: float, min_value: float, max_value: float, step: float, callback: Callable) -> void:
	var label := Label.new()
	label.text = label_text
	_inspector_body.add_child(label)
	var spin := SpinBox.new()
	spin.min_value = min_value
	spin.max_value = max_value
	spin.step = step
	spin.value = value
	# Use focus_exited so we don't export on every arrow-key tick.
	spin.get_line_edit().focus_exited.connect(func(): callback.call(spin.value); _commit())
	spin.value_changed.connect(func(new_value): callback.call(new_value))
	_inspector_body.add_child(spin)

func _add_read_only_block(label_text: String, value: String) -> void:
	var label := Label.new()
	label.text = label_text
	_inspector_body.add_child(label)
	var block := RichTextLabel.new()
	block.custom_minimum_size.y = 96
	block.bbcode_enabled = false
	block.text = value
	_inspector_body.add_child(block)

func _refresh_diagnostics() -> void:
	_diagnostics_label.clear()
	var lines: Array[String] = []
	lines.append("Validation")
	if _validation.is_empty():
		lines.append("- clean")
	else:
		for message in _validation:
			lines.append("- [%s] %s: %s" % [
				str(message.get("severity", "info")).to_upper(),
				str(message.get("scope", "")),
				str(message.get("message", "")),
			])
	lines.append("")
	lines.append("Sandbox")
	if not _sandbox_error.is_empty():
		lines.append("- last error: %s" % _sandbox_error)
	if _sandbox_result.is_empty():
		lines.append("- no sandbox run yet")
	else:
		lines.append("- scenario: %s" % str(_sandbox_result.get("display_name", _sandbox_result.get("scenario_id", ""))))
		lines.append("- daily consumption: %.1f units/day" % float(_sandbox_result.get("daily_household_demand_units", 0.0)))
		lines.append("- end-of-run supplies: %.2f days" % float(_sandbox_result.get("final_household_stock_days", 0.0)))
		lines.append("- lowest supplies reached: %.2f days" % float(_sandbox_result.get("lowest_household_stock_days", 0.0)))
		lines.append("- total undelivered: %.0f units" % float(_sandbox_result.get("total_unmet_units", 0.0)))
		lines.append("- avg cost per household/day: %.2f" % float(_sandbox_result.get("average_household_cost_per_day", 0.0)))
		for bottleneck in _sandbox_result.get("bottlenecks", []):
			lines.append("- bottleneck: %s" % str(bottleneck))
		var daily: Array = _sandbox_result.get("daily", [])
		if not daily.is_empty():
			lines.append("- last day: supplies %.2f, unmet %.2f" % [
				float(daily[daily.size() - 1].get("household_stock_days", 0.0)),
				float(daily[daily.size() - 1].get("unmet_units", 0.0)),
			])
	_diagnostics_label.text = "\n".join(lines)

func _update_selected_entry(value, field: String) -> void:
	var section := "%ss" % _selected_kind
	var index := _find_entry_index(section, _selected_id)
	if index == -1:
		return
	var items: Array = _project.get(section, [])
	var entry: Dictionary = items[index]
	entry[field] = value
	items[index] = entry
	_project[section] = items
	# Only rebuild the browser tree when display_name changes — it's the only
	# field visible there. Rebuilding on every keystroke steals keyboard focus.
	if field == "display_name":
		_rebuild_browser()
		_rebuild_scenario_picker()
	_refresh_graph()

func _commit() -> void:
	if _project.is_empty():
		return
	var payload := _parse_json(sim.export_economy_project(JSON.stringify(_project), _economy_dir))
	_validation = payload.get("validation", [])
	_refresh_diagnostics()
	if not payload.get("ok", false):
		_set_status("Auto-save failed: %s" % str(payload.get("error", "")), true)
	else:
		_set_status("Saved")

func _update_port_value(value: float, port_list_key: String, port_index: int) -> void:
	var section := "profiles"
	var index := _find_entry_index(section, _selected_id)
	if index == -1:
		return
	var items: Array = _project.get(section, [])
	var entry: Dictionary = items[index].duplicate()
	var ports: Array = entry.get(port_list_key, []).duplicate()
	if port_index >= 0 and port_index < ports.size():
		var port: Dictionary = ports[port_index].duplicate()
		port["units_per_day"] = value
		ports[port_index] = port
		entry[port_list_key] = ports
		items[index] = entry
		_project[section] = items
		_rebuild_browser()
		_refresh_graph()

func _update_selected_node_position(value: float, axis: int) -> void:
	if _selected_node_id.is_empty():
		return
	var scenarios: Array = _project.get("scenarios", [])
	for scenario_index in range(scenarios.size()):
		if str(scenarios[scenario_index].get("id", "")) != _current_scenario_id:
			continue
		var nodes: Array = scenarios[scenario_index].get("nodes", [])
		for node_index in range(nodes.size()):
			if str(nodes[node_index].get("id", "")) != _selected_node_id:
				continue
			var position: Array = nodes[node_index].get("position", [0.0, 0.0])
			position[axis] = value
			nodes[node_index]["position"] = position
			scenarios[scenario_index]["nodes"] = nodes
			_project["scenarios"] = scenarios
			_refresh_graph()
			return

func _on_browser_item_selected() -> void:
	var item := _browser_tree.get_selected()
	if item == null:
		return
	var metadata = item.get_metadata(0)
	if metadata == null:
		return
	_select_item(str(metadata.get("kind", "")), str(metadata.get("id", "")), "")

func _on_scenario_selected(index: int) -> void:
	if index < 0 or index >= _scenario_ids.size():
		return
	var next_scenario_id := _scenario_ids[index]
	if next_scenario_id == _current_scenario_id:
		return
	_current_scenario_id = next_scenario_id
	_sandbox_result = {}
	_sandbox_error = ""
	_refresh_graph()
	_refresh_diagnostics()
	if _selected_kind == "scenario":
		_select_item("scenario", _current_scenario_id, "")

func _on_graph_node_selected(node_id: String) -> void:
	var node := _find_node_in_current_scenario(node_id)
	if node.is_empty():
		return
	_select_item(str(node.get("ref_kind", "")), str(node.get("ref_id", "")), node_id)

func _on_graph_node_moved(node_id: String, position: Vector2) -> void:
	var scenarios: Array = _project.get("scenarios", [])
	for scenario_index in range(scenarios.size()):
		if str(scenarios[scenario_index].get("id", "")) != _current_scenario_id:
			continue
		var nodes: Array = scenarios[scenario_index].get("nodes", [])
		for node_index in range(nodes.size()):
			if str(nodes[node_index].get("id", "")) != node_id:
				continue
			nodes[node_index]["position"] = [position.x, position.y]
			scenarios[scenario_index]["nodes"] = nodes
			_project["scenarios"] = scenarios
			if node_id == _selected_node_id:
				_refresh_inspector()
			return

func _select_item(kind: String, id: String, node_id: String) -> void:
	_selected_kind = kind
	_selected_id = id
	_selected_node_id = node_id
	if kind == "scenario":
		_current_scenario_id = id
		_rebuild_scenario_picker()
	_refresh_graph()
	_refresh_inspector()

func _find_entry(section: String, id: String) -> Dictionary:
	var entries: Array = _project.get(section, [])
	for entry in entries:
		if str(entry.get("id", "")) == id:
			return entry
	return {}

func _find_entry_index(section: String, id: String) -> int:
	var entries: Array = _project.get(section, [])
	for index in range(entries.size()):
		if str(entries[index].get("id", "")) == id:
			return index
	return -1

func _find_graph_node_for_ref(ref_kind: String, ref_id: String) -> String:
	var scenario := _get_selected_scenario()
	for node in scenario.get("nodes", []):
		if str(node.get("ref_kind", "")) == ref_kind and str(node.get("ref_id", "")) == ref_id:
			return str(node.get("id", ""))
	return ""

func _find_node_in_current_scenario(node_id: String) -> Dictionary:
	var scenario := _get_selected_scenario()
	for node in scenario.get("nodes", []):
		if str(node.get("id", "")) == node_id:
			return node
	return {}

func _get_selected_scenario() -> Dictionary:
	return _find_entry("scenarios", _current_scenario_id)

func _ports_summary(profile: Dictionary) -> String:
	var lines: Array[String] = []
	lines.append("Inputs")
	var inputs: Array = profile.get("inputs", [])
	if inputs.is_empty():
		lines.append("- none")
	else:
		for port in inputs:
			lines.append("- %s: %.2f/day" % [str(port.get("resource", "")), float(port.get("units_per_day", 0.0))])
	lines.append("")
	lines.append("Outputs")
	var outputs: Array = profile.get("outputs", [])
	if outputs.is_empty():
		lines.append("- none")
	else:
		for port in outputs:
			lines.append("- %s: %.2f/day" % [str(port.get("resource", "")), float(port.get("units_per_day", 0.0))])
	return "\n".join(lines)

func _parse_json(raw: String) -> Dictionary:
	var parsed = JSON.parse_string(raw)
	if parsed == null or not (parsed is Dictionary):
		return {
			"ok": false,
			"error": "could not parse JSON payload",
			"validation": [],
		}
	return parsed

func _set_status(message: String, is_error: bool = false) -> void:
	_status_label.text = message
	if is_error:
		_status_label.add_theme_color_override("font_color", Color(0.92, 0.48, 0.48))
	else:
		_status_label.add_theme_color_override("font_color", Color(0.76, 0.86, 0.78))
