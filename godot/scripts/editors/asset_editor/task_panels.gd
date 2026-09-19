# SPDX-License-Identifier: GPL-2.0-only

## Task-specific building inspector construction. Applicability comes from Rust via the session.
## No pack I/O, simulation decisions or document mutation is owned by this view builder.
extends RefCounted

var v: RefCounted
var e: Node

func build(view: RefCounted) -> void:
	v = view
	e = view._editor
	_overview()
	_model()
	_site()
	_gameplay()
	_validate()

func _label(parent: Control, text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	parent.add_child(label)
	return label

func _overview() -> void:
	var box: Control = v.tasks["overview"]
	v.type_label = _label(box, "Create or open an asset to begin.")
	v.button(box, "Change asset type…", e._session.open_conversion)
	v._display_name_edit = v.text_field(box, "display_name", "Name · authored")
	v._pack_summary_lbl = _label(box, "No destination pack")
	v._pack_set_btn = v.button(box, "Choose / manage pack…", func(): e._open_pack_select_menu(v._pack_set_btn))
	v._pack_select_menu = PopupMenu.new()
	box.add_child(v._pack_select_menu)
	v._pack_select_menu.id_pressed.connect(e._on_pack_select_menu_id_pressed)
	v.thumbnail = TextureRect.new()
	v.thumbnail.custom_minimum_size = Vector2(128, 96)
	v.thumbnail.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	v.thumbnail.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
	box.add_child(v.thumbnail)
	v.button(box, "Capture thumbnail from preview", e._session.capture_thumbnail)
	v.status = _label(box, "")
	var advanced: Control = v.advanced(box, "identity")
	v._asset_id_edit = v.text_field(advanced, "asset_id", "Asset ID · authored")
	v.text_field(advanced, "tags", "Tags · comma-separated")
	v.button(box, "Save draft as…", e._session.save_draft_as)

func _model() -> void:
	var box: Control = v.tasks["model"]
	var actions := HFlowContainer.new()
	box.add_child(actions)
	v.button(actions, "Import mesh…", e._on_import_glb_pressed)
	v.remove_part_button = v.geometry_button(actions, "Remove part", e._remove_selected_mesh_parts)
	v.model_hint = _label(box, "Import a model to begin. Add its LOD meshes after selecting a part.")
	v._mesh_part_list = ItemList.new()
	v._mesh_part_list.custom_minimum_size.y = 72
	v._mesh_part_list.select_mode = ItemList.SELECT_MULTI
	v._mesh_part_list.multi_selected.connect(e._on_mesh_part_multi_selected)
	box.add_child(v._mesh_part_list)
	v.part_properties = VBoxContainer.new()
	box.add_child(v.part_properties)
	var part: Control = v.part_properties
	v._dim_label = _label(part, "Select a mesh part.")
	v._preview_panel.move_lod_controls(part)
	part = v.advanced(v.part_properties, "placement", "Placement")
	v._preview_scale_spin = _geometry_number(part, "_part_scale", "Scale · authored", 0.001, 1000, 0.01, e._on_part_transform_changed.bind("scale"))
	v._part_x_spin = _geometry_number(part, "_part_x", "Position X (m)", -500, 500, 0.1, e._on_part_transform_changed.bind("x"))
	v._part_y_spin = _geometry_number(part, "_part_y", "Position Y (m)", -500, 500, 0.1, e._on_part_transform_changed.bind("y"))
	v._part_z_spin = _geometry_number(part, "_part_z", "Position Z (m)", -500, 500, 0.1, e._on_part_transform_changed.bind("z"))
	v._part_rotation_y_spin = _geometry_number(part, "_part_yaw", "Rotation Y (°)", -180, 180, 1, e._on_part_transform_changed.bind("yaw"))
	v.geometry_button(part, "Fit selected part to lot", e._on_autofit_pressed)
	v.part_properties.visible = false

func _site() -> void:
	var box: Control = v.tasks["site"]
	v.site_picker = OptionButton.new()
	for label in ["Footprint & frontage", "Access points", "Surfaces"]:
		v.site_picker.add_item(label)
	box.add_child(v.site_picker)
	v.site_picker.item_selected.connect(v.show_site)
	for index in 3:
		var body := VBoxContainer.new()
		box.add_child(body)
		v.site_sections.append(body)
	var footprint: Control = v.site_sections[0]
	v._width_spin = v.number_field(footprint, "lot_width_cells", "Lot width (10 m cells) · authored", 65535)
	v._depth_spin = v.number_field(footprint, "lot_depth_cells", "Lot depth (10 m cells) · authored", 65535)
	v._frontage_lbl = _label(footprint, "Frontage")
	v.geometry_button(footprint, "Set frontage from view", e._on_set_front_from_view)
	v.geometry_button(footprint, "Move main entrance to frontage", e._on_reset_main_entrance_pressed)
	var advanced: Control = v.advanced(footprint, "footprint")
	v.number_field(advanced, "min_zone_width_cells", "Minimum zoned width (cells)", 65535)
	v.number_field(advanced, "min_zone_depth_cells", "Minimum zoned depth (cells)", 65535)
	_access(v.site_sections[1])
	_surfaces(v.site_sections[2])
	v.show_site(0)

func _access(box: Control) -> void:
	var actions := HFlowContainer.new()
	box.add_child(actions)
	for kind in ["entrance", "driveway", "parking", "loading_bay"]:
		v.geometry_button(actions, kind.capitalize().replace("_", " "), e._add_site_anchor.bind(kind))
	v._site_anchor_list = ItemList.new()
	v._site_anchor_list.custom_minimum_size.y = 125
	v._site_anchor_list.select_mode = ItemList.SELECT_MULTI
	v._site_anchor_list.multi_selected.connect(e._on_site_anchor_multi_selected)
	box.add_child(v._site_anchor_list)
	v.remove_anchor_button = v.geometry_button(box, "Remove selected access point", e._remove_selected_site_anchor)
	v.anchor_properties = VBoxContainer.new()
	box.add_child(v.anchor_properties)
	var body: Control = v.anchor_properties
	v._site_anchor_name_edit = v.text_field(body, "_anchor_name", "Name · authored")
	v._site_anchor_name_edit.text_changed.connect(e._on_site_anchor_text_changed)
	v._site_anchor_name_edit.text_changed.connect(func(_text): e._session.capture_geometry("Rename access point"))
	v._site_anchor_vehicle_class_btn = v.choice_field(body, "_anchor_vehicle", "Vehicle class", ["car", "freight", "service"])
	v._site_anchor_vehicle_class_btn.item_selected.connect(e._on_site_anchor_vehicle_class_selected)
	v._site_anchor_vehicle_class_btn.item_selected.connect(func(_index): e._session.capture_geometry("Change access class"))
	v._site_anchor_x_spin = _geometry_number(body, "_anchor_x", "Position X (m)", -500, 500, 0.1, e._on_site_anchor_spin_changed.bind("x"))
	v._site_anchor_y_spin = _geometry_number(body, "_anchor_y", "Position Y (m)", -500, 500, 0.1, e._on_site_anchor_spin_changed.bind("y"))
	v._site_anchor_z_spin = _geometry_number(body, "_anchor_z", "Position Z (m)", -500, 500, 0.1, e._on_site_anchor_spin_changed.bind("z"))
	v._site_anchor_yaw_spin = _geometry_number(body, "_anchor_yaw", "Rotation Y (°)", -180, 180, 0.1, e._on_site_anchor_spin_changed.bind("yaw"))
	v._site_anchor_width_spin = _geometry_number(body, "_anchor_width", "Width (m)", 0, 100, 0.1, e._on_site_anchor_spin_changed.bind("width_m"))
	v._site_anchor_length_spin = _geometry_number(body, "_anchor_length", "Length (m)", 0, 100, 0.1, e._on_site_anchor_spin_changed.bind("length_m"))
	v.anchor_properties.visible = false

func _surfaces(box: Control) -> void:
	var actions := HFlowContainer.new()
	box.add_child(actions)
	for material in ["asphalt", "concrete"]:
		v.geometry_button(actions, "Add " + material, e._add_site_surface.bind(material))
	v._site_surface_list = ItemList.new()
	v._site_surface_list.custom_minimum_size.y = 125
	v._site_surface_list.item_selected.connect(e._on_site_surface_selected)
	box.add_child(v._site_surface_list)
	v.remove_surface_button = v.geometry_button(box, "Remove surface", e._remove_selected_site_surface)
	v.surface_properties = VBoxContainer.new()
	box.add_child(v.surface_properties)
	var body: Control = v.surface_properties
	v._site_surface_name_edit = v.text_field(body, "_surface_name", "Name · authored")
	v._site_surface_name_edit.text_changed.connect(e._on_site_surface_text_changed)
	v._site_surface_name_edit.text_changed.connect(func(_value): e._session.capture_geometry("Rename surface"))
	v._site_surface_material_btn = v.choice_field(body, "_surface_material", "Material", ["asphalt", "concrete"])
	v._site_surface_material_btn.item_selected.connect(e._on_site_surface_material_selected)
	v._site_surface_material_btn.item_selected.connect(func(_value): e._session.capture_geometry("Change surface material"))
	v._site_surface_y_spin = _geometry_number(body, "_surface_y", "Height (m)", -50, 50, 0.01, e._on_site_surface_spin_changed)
	_label(body, "Drag vertices in the viewport. Right-click an edge to add a vertex, or a vertex to remove it.")
	v._site_surface_context_menu = PopupMenu.new()
	v._site_surface_context_menu.id_pressed.connect(e._on_site_surface_context_menu_id_pressed)
	box.add_child(v._site_surface_context_menu)
	v.surface_properties.visible = false

func _gameplay() -> void:
	var box: Control = v.tasks["gameplay"]
	_label(box, "Authored settings for this asset type")
	v.choice_field(box, "density", "Density")
	v._residents_spin = v.number_field(box, "household_capacity", "Households", 1000000)
	v.number_field(box, "flat_size_m2", "Apartment / farmhouse area (m²)", 100000, 0.5)
	v.number_field(box, "worker_capacity", "Employment · authored", 1000000)
	v.text_field(box, "extractor_resource", "Extracted resource")
	v.text_field(box, "field_resource", "Grown resource")
	v.field_row(box, "extractor_area_mode", "Extraction area · runtime contract", _label_detached("Player-drawn polygon"))
	v.field_row(box, "field_area_mode", "Field area · runtime contract", _label_detached("Player-drawn polygon; one farmhouse household"))
	v.field_row(box, "service_class", "Service subtype", _label_detached(""))
	v._economy_profile_btn = v.choice_field(box, "economy_profile", "Compatible economy profile")
	v.derived_workers = _label(box, "")
	v._economy_profile_status_lbl = _label(box, "")
	v.catalog_refresh = v.button(box, "Refresh catalog", e._session.refresh_catalog)
	var advanced: Control = v.advanced(box, "gameplay")
	v.text_field(advanced, "asset_set", "Upgrade family")
	v.number_field(advanced, "level", "Level in upgrade family", 255)

func _validate() -> void:
	var box: Control = v.tasks["validate"]
	_label(box, "Drafts can be incomplete. Runtime export requires all errors to be resolved.")
	v.export_summary = _label(box, "")
	v.button(box, "Revalidate", e._session.validate)
	v.issues_box = VBoxContainer.new()
	box.add_child(v.issues_box)
	v.button(box, "Export runtime asset…", e._on_export_pressed)

func _label_detached(text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	return label

func _geometry_number(parent: Node, key: String, title: String, minimum: float, maximum: float, step: float, callback: Callable) -> SpinBox:
	var spin: SpinBox = v.number_field(parent, key, title, maximum, step)
	spin.min_value = minimum
	spin.allow_lesser = true
	spin.value_changed.connect(callback)
	spin.value_changed.connect(func(_value):
		if not e._session.rendering:
			e._session.capture_geometry(title)
	)
	return spin
