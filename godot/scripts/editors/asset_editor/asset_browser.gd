# SPDX-License-Identifier: GPL-3.0-or-later

## Asset library tree view (`TOOLS-34`): draws the packs, groups and rows Rust returns from
## `get_asset_browser_rows`, already grouped, ordered and searched. One row per building, with
## one level chip per family member. A chip is a Tree button whose transparent texture reserves
## its place and whose click Tree's own hit test reports; an overlay above the rows paints the
## labels, since Tree draws its rows over anything a custom cell callback draws once its opaque
## focus style is shown. This script holds no grouping, ordering or search logic.
extends RefCounted

const CHIP_PAD_X := 5.0
const MISSING_SECTION := "No assets yet"

var tree: Tree
## Qualified id -> row label with its level chip (`House 1 · L2`), for menus and dialogs.
var labels := {}
var _chips := {}  # TreeItem -> Array of {id, text}, in chip (button id) order
var _chip_textures := {}  # Vector2i -> transparent ImageTexture of that size
var _chip_style := StyleBoxFlat.new()
var _overlay := Control.new()

func _init(target: Tree) -> void:
	tree = target
	_chip_style.set_corner_radius_all(3)
	_chip_style.set_border_width_all(1)
	# Clipped to the tree's content area, so a chip half scrolled out is cut at the panel edge.
	_overlay.name = "LevelChips"
	_overlay.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_overlay.clip_contents = true
	_overlay.set_anchors_preset(Control.PRESET_FULL_RECT)
	tree.add_child(_overlay)
	tree.draw.connect(_overlay.queue_redraw)
	_overlay.draw.connect(_draw_chips)

## Rebuilds the label map from the unsearched rows.
func index_labels(result: Dictionary) -> void:
	labels.clear()
	for pack: Dictionary in result.get("packs", []):
		for group: Dictionary in pack.groups:
			for row: Dictionary in group.rows:
				for level: Dictionary in row.levels:
					labels[level.id] = _level_label(row.label, int(level.level))

## Rebuilds the tree. `known_packs` (sorted pack manifests) also list packs without assets while
## no search is given. Returns the number of matching assets.
func build(result: Dictionary, known_packs: Array, searching: bool) -> int:
	tree.clear()
	_chips.clear()
	var panel := tree.get_theme_stylebox("panel")
	_overlay.offset_left = panel.get_margin(SIDE_LEFT)
	_overlay.offset_top = panel.get_margin(SIDE_TOP)
	_overlay.offset_right = -panel.get_margin(SIDE_RIGHT)
	_overlay.offset_bottom = -panel.get_margin(SIDE_BOTTOM)
	var root := tree.create_item()
	var packs: Array = result.get("packs", [])
	var by_pack := {}
	var visible := 0
	for pack: Dictionary in packs:
		by_pack[pack.pack] = pack
		visible += int(pack.asset_count)
	if packs.is_empty():
		var empty := tree.create_item(root)
		empty.set_text(0, "No matching assets")
		empty.set_selectable(0, false)
	var order: Array = []
	if not searching:
		for known: Dictionary in known_packs:
			order.append(str(known.pack_id))
	for pack: Dictionary in packs:
		if not order.has(pack.pack):
			order.append(pack.pack)
	for pack_id: String in order:
		_add_pack(root, pack_id, by_pack.get(pack_id, {}))
	var missing: Array = result.get("missing", [])
	if not missing.is_empty():
		var section := _heading(root, "%s (%d)" % [MISSING_SECTION, missing.size()], null)
		for group: Dictionary in missing:
			var item := _heading(section, "%s (0)" % group.label, null)
			item.set_custom_color(0, tree.get_theme_color("font_disabled_color"))
			item.set_tooltip_text(0, "No pack has an asset in this group.")
	return visible

## The qualified id behind chip `index` of `item`, or "".
func level_id(item: TreeItem, index: int) -> String:
	var chips: Array = _chips.get(item, [])
	return str(chips[index].id) if index >= 0 and index < chips.size() else ""

func _add_pack(root: TreeItem, pack_id: String, pack: Dictionary) -> void:
	var pack_item := _heading(root, "%s (%d)" % [pack_id, int(pack.get("asset_count", 0))], {"pack": pack_id})
	for group: Dictionary in pack.get("groups", []):
		var group_item := _heading(pack_item, "%s (%d)" % [group.label, int(group.asset_count)], {"pack": pack_id, "type": group.type})
		for row: Dictionary in group.rows:
			_add_row(group_item, row)

func _heading(parent: TreeItem, text: String, metadata: Variant) -> TreeItem:
	var item := tree.create_item(parent)
	item.set_text(0, text)
	item.set_selectable(0, false)
	item.set_metadata(0, metadata)
	return item

# The row opens its first (lowest) level; each building level is a chip button whose id is its
# index in the row's level list.
func _add_row(parent: TreeItem, row: Dictionary) -> void:
	var levels: Array = row.levels
	var item := tree.create_item(parent)
	item.set_text(0, row.label)
	item.set_metadata(0, levels[0].id)
	var tooltip: String = levels[0].id
	if not str(row.asset_set).is_empty():
		tooltip += "\nFamily: " + str(row.asset_set)
	item.set_tooltip_text(0, tooltip)
	var chips: Array = []
	for level: Dictionary in levels:
		if int(level.level) <= 0:
			continue
		var text := "L%d" % int(level.level)
		item.add_button(0, _chip_texture(text), chips.size(), false, "Level %d · %s" % [int(level.level), level.id])
		chips.append({"id": level.id, "text": text})
	if not chips.is_empty():
		_chips[item] = chips

func _chip_size(chip: String) -> Vector2i:
	var font := tree.get_theme_font("font")
	var font_size := tree.get_theme_font_size("font_size")
	var text := font.get_string_size(chip, HORIZONTAL_ALIGNMENT_LEFT, -1, font_size)
	return Vector2i(ceili(text.x + 2.0 * CHIP_PAD_X), ceili(font.get_height(font_size)))

func _chip_texture(chip: String) -> Texture2D:
	var size := _chip_size(chip)
	if not _chip_textures.has(size):
		var image := Image.create_empty(size.x, size.y, false, Image.FORMAT_RGBA8)
		_chip_textures[size] = ImageTexture.create_from_image(image)
	return _chip_textures[size]

# Paints each visible chip's outline and label inside its button's rectangle, after Tree drew
# the button (its transparent texture, hover and pressed styles). O(chip rows) per tree redraw.
func _draw_chips() -> void:
	var font := tree.get_theme_font("font")
	var font_size := tree.get_theme_font_size("font_size")
	var color := tree.get_theme_color("font_color")
	_chip_style.bg_color = Color(color, 0.08)
	_chip_style.border_color = Color(color, 0.45)
	var visible := Rect2(Vector2.ZERO, _overlay.size)
	for item: TreeItem in _chips:
		if _folded(item):
			continue
		var chips: Array = _chips[item]
		for index in chips.size():
			var text: String = chips[index].text
			var area := tree.get_item_area_rect(item, 0, index)
			area.position -= _overlay.position
			if not visible.intersects(area):
				continue
			var size := Vector2(_chip_size(text))
			var chip := Rect2(area.get_center() - size * 0.5, size)
			_overlay.draw_style_box(_chip_style, chip)
			var baseline := chip.position.y + (chip.size.y - font.get_height(font_size)) * 0.5 + font.get_ascent(font_size)
			_overlay.draw_string(font, Vector2(chip.position.x, baseline), text, HORIZONTAL_ALIGNMENT_CENTER, chip.size.x, font_size, color)

# Whether a collapsed ancestor hides the row.
func _folded(item: TreeItem) -> bool:
	var parent := item.get_parent()
	while parent != null:
		if parent.collapsed:
			return true
		parent = parent.get_parent()
	return false

func _level_label(label: String, level: int) -> String:
	return "%s · L%d" % [label, level] if level > 0 else label
