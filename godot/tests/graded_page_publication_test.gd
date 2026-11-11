# SPDX-License-Identifier: GPL-2.0-only

## Exercises full-page migration and edit publication with controlled allocator pressure.
## GPU runs compare the published bytes (Compatibility: the pages' CPU copy, TERRAIN-22);
## headless runs check ownership and serials.
extends SceneTree

const Tiles := preload("res://scripts/renderers/terrain_graded_tiles.gd")
const Pages := preload("res://scripts/renderers/terrain_graded_pages.gd")
var failures := 0

func _initialize() -> void:
	call_deferred("_run")

func expect(condition: bool, message: String) -> void:
	if not condition:
		failures += 1
		push_error(message)

func payload(graded: Tiles, key: Vector2i, token: int, layers: int) -> Dictionary:
	var side := int(graded.page_store.levels[2]["side"])
	var texels := Image.create(side, side + 1, false, Image.FORMAT_RGBAF)
	texels.fill(Color(float(token), 2, 3, 4))
	texels.set_pixel(0, side, Color(0, 2, 3, 4))
	var atlas := Image.create(Pages.ATLAS_WIDTH, layers, false, Image.FORMAT_RGBAF)
	atlas.fill(Color(float(token), 6, 7, 8))
	return {"tile_x": key.x, "tile_z": key.y, "level": 2, "step": 0, "token": token,
		"texels": texels.get_data(), "rows": atlas.get_data(), "walls": PackedByteArray(), "behind": false}

func check_bytes(graded: Tiles, entry: Dictionary, data: Dictionary) -> void:
	if DisplayServer.get_name() == "headless":
		return
	for page_id: int in entry["copies"]:
		var page: Dictionary = graded.page_store.by_id[page_id]
		var copy: Vector3i = entry["copies"][page_id]
		var side := int(graded.page_store.levels[int(page["code"])]["side"])
		var image := Image.create_from_data(side, side + 1, false, Image.FORMAT_RGBAF, graded.page_store.layer_data(page, true, copy.x))
		var header := image.get_pixel(0, image.get_height() - 1)
		header.r = 0
		image.set_pixel(0, image.get_height() - 1, header)
		expect(image.get_data() == data["texels"], "migration preserves exact slot bytes")
		var rows := PackedByteArray()
		for layer in (data["rows"] as PackedByteArray).size() / Pages.ATLAS_LAYER_BYTES:
			rows.append_array(graded.page_store.layer_data(page, false, copy.y + layer))
		expect(rows == data["rows"], "migration preserves exact atlas bytes")

func _run() -> void:
	var sim := SimulationNode.new()
	root.add_child(sim)
	expect(sim.create_blank_world(500, 500, 10, 512, 0), "fixture world")
	sim.set_simulation_speed(0)
	var graded := Tiles.new()
	graded.setup(sim, 10, Vector2(500, 500), 512, [8, 8, 8, 8])
	graded._resident = {Vector2i.ZERO: {"rect": Rect2(-250, -250, 512, 512), "step": 1, "generation": 0}}
	graded._book.sync_resident(PackedInt32Array([0, 0]))
	graded._resident_synced = true
	graded.patch_lists[Vector2i.ZERO] = {"tiles": [], "token": 0}
	graded._book.take_list(Vector2i.ZERO, 0, PackedInt32Array())
	var a := Vector2i.ZERO
	var b := Vector2i(1, 0)
	var first := payload(graded, a, 1, 12)
	var second := payload(graded, b, 1, 12)
	graded._upload(a, first, graded._resident)
	graded._show_preview_tile(second)
	var old_id := int(graded._book.block_page(2, Vector2i.ZERO))
	expect(int(graded._book.page_info(old_id)[6]) == 0, "fixture fills the old atlas")
	expect(graded._book.level_of(a).x < 0, "commit-only installed tile has no residency-list reference")
	# Same preview epoch, newer pose: its failed upload must not become adoptable old bytes.
	var newer_preview := payload(graded, b, 1, 13)
	graded._show_preview_tile(newer_preview)
	var adopted := newer_preview.duplicate()
	adopted["adopted"] = true
	adopted["preview_epoch"] = 1
	expect(not graded._adopt_preview(b, adopted), "a failed latest preview upload cannot adopt the preceding pose")
	# The edit needs a larger atlas run than the full bound page can provide. It must wait
	# without acknowledgement, while migration preserves both installed and preview holders.
	var changed := payload(graded, a, 2, 13)
	graded._edit_serial_taken = 2
	graded._book.edit_batch_set(a, 2)
	graded._edit_staged[a] = changed
	expect(graded._install_edit_batch(graded._resident) == 0, "full bound page defers the whole edit")
	expect(graded.installed_edit_serial() < 2, "a deferred upload must not acknowledge its edit")
	expect(int(graded.installed[a]["token"]) == 1, "deferred edit preserves published token")
	graded._advance_growth()
	expect(graded._book.block_target(2, Vector2i.ZERO) < 0, "unlisted installed tiles cannot strand migration")
	expect(graded._book.block_page(2, Vector2i.ZERO) != old_id, "larger page becomes bound")
	await process_frame
	check_bytes(graded, graded.installed[a], first)
	check_bytes(graded, graded.preview[b], second)
	expect(graded._install_edit_batch(graded._resident) == 1, "edit installs after the page switches")
	expect(graded.installed_edit_serial() == 2, "only installed edit is acknowledged")
	await process_frame
	check_bytes(graded, graded.installed[a], changed)
	# A later site/road invalidation must rebake a commit-only tile too. It is still visible
	# even though no patch list has ever named it.
	graded._dirty_tile(a, 3)
	graded._request_tiles()
	expect(graded._book.requested_token(a, 2, 0) >= 0, "unlisted installed tile requests its dirty rebake")
	# A site edit can arrive while a new road tile is staged but not installed yet.
	var c := Vector2i(2, 0)
	graded._book.edit_batch_set(c, 3)
	graded._edit_staged[c] = payload(graded, c, 3, 1)
	graded._apply_edit({"serial": 4, "all": false, "levels": [{"level": 2, "step": 0, "tiles": PackedInt32Array([2, 0])}]})
	graded._request_tiles()
	expect(graded._book.edit_batch_token(c) == 4 and graded._book.requested_token(c, 2, 0) == 4,
		"a newer site edit rebakes a tile invalidated before its first install")
	graded._book.undesire(c)
	graded._apply_effects()
	graded.release_preview(0, 1)
	graded._resident = {}
	graded._book.sync_resident(PackedInt32Array())
	graded._patch_changed(Vector2i.ZERO, {})
	expect(graded.installed.is_empty() and graded._book.edit_batch_len() == 0, "leaving residency evicts commit-only tiles and pending edits")
	graded.dispose()
	sim.free()
	print("Graded page publication tests: %d failures" % failures)
	quit(0 if failures == 0 else 1)
