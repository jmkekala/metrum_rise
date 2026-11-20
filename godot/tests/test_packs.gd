# SPDX-License-Identifier: GPL-3.0-or-later

## The pack folder tests load when they need the test building pack beside a bundled pack.
##
## `load_asset_packs` reads one folder, and the test pack (res://tests/mods) and the bundled packs
## (res://bootstrap/mods) live apart, so this copies both into the test profile's user:// once.
extends RefCounted

const UserDataBootstrap = preload("res://scripts/core/user_data_bootstrap.gd")
const ROOT := "user://test-packs"

static func mods_root() -> String:
	UserDataBootstrap._seed_top_level_entries("res://tests/mods", ROOT)
	UserDataBootstrap._seed_top_level_entries(UserDataBootstrap.BOOTSTRAP_MODS_DIR, ROOT)
	return ProjectSettings.globalize_path(ROOT)
