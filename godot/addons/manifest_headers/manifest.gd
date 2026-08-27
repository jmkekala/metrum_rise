# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: manifest.gd
#  script_path: addons/manifest_headers/manifest.gd
#  module_name: manifest
#  version: 1.0.0
#  description: The header engine. Reads whatever header a file already has,
#           writes a standard one in the comment syntax that file's
#           extension wants, and expands a bare HEADER marker into a full
#           section divider. Written against the parser the file browser
#           actually runs rather than the prose describing it, because that
#           parser decides whether a file is visible at all and a file whose
#           header will not parse fails silently.
#  kind: module
#  spec: none
#  internal_dependencies: []
#  external_dependencies: [Godot 4.x]
#  features: [read-header, write-header, expand-marker, custom-fields, check-mode, tag-normalisation]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-24
# =========================================================================

@tool
extends RefCounted
class_name ManifestHeaders

# =========================================================================
# THE CONTRACT THIS SATISFIES
# =========================================================================
# The file browser's parser is stricter than its documentation suggests, and
# three rules decide whether a file appears at all:
#
#   1. THE HEADER LIVES IN THE FIRST 40 LINES. The parser reads only the
#      first forty and nothing below them exists.
#   2. THE WORD MANIFEST MUST APPEAR IN THOSE LINES, or the parse returns
#      nothing and the file falls back to synthesised metadata.
#   3. ONE FIELD PER LINE. Values are read by a per-line regex, so a value
#      carrying a quote or a bracket does not survive the comment form.
#
# A header that breaks those rules does not raise an error. The file quietly
# drops out of the browser, the drift audit, and the build facts, which is
# why this ships a check mode a CI run can fail on.

const DIVIDER_WIDTH := 75

## Fields every header carries, in the order they are written.
const BASE_FIELDS: Array[String] = [
	"script_name",
	"script_path",
	"module_name",
	"version",
	"description",
	"kind",
	"spec",
	"internal_dependencies",
	"external_dependencies",
	"features",
	"api_version",
	"last_updated",
]

## Fields written as [a, b, c] rather than a bare value.
const LIST_FIELDS: Array[String] = [
	"internal_dependencies",
	"external_dependencies",
	"features",
]

## Comment leader per extension. An extension absent here is not touched,
## which is deliberate: a file this tool does not understand is left alone
## rather than mangled.
const COMMENT_LEADER := {
	"gd": "#",
	"rs": "//",
	"py": "#",
	"sh": "#",
	"ps1": "#",
	"toml": "#",
	"yml": "#",
	"yaml": "#",
	"cfg": "#",
	"ts": "//",
	"tsx": "//",
	"js": "//",
	"mjs": "//",
	"kt": "//",
	"c": "//",
	"cpp": "//",
	"h": "//",
	"hpp": "//",
	"gdshader": "//",
}

const CONFIG_PATH := "res://addons/manifest_headers/manifest_fields.cfg"


# =========================================================================
# CONFIG
# =========================================================================
# A project declares its extra fields once and every header written after
# that carries them. The point is that a field is tracked because it was
# declared, not because somebody remembered to type it.

class Config extends RefCounted:
	var custom_fields: Array[String] = []
	var list_fields: Array[String] = []
	var api_version: String = ""
	var defaults: Dictionary = {}

	## Every field name in write order: the base set, then the project's own.
	func field_order() -> Array[String]:
		var out: Array[String] = BASE_FIELDS.duplicate()
		for f in custom_fields:
			if not out.has(f):
				out.append(f)
		return out

	func is_list_field(name: String) -> bool:
		return LIST_FIELDS.has(name) or list_fields.has(name)


static func load_config(path: String = CONFIG_PATH) -> Config:
	var cfg := Config.new()
	var file := ConfigFile.new()
	if file.load(path) != OK:
		return cfg
	cfg.api_version = String(file.get_value("manifest", "api_version", ""))
	for f in file.get_value("manifest", "custom_fields", []):
		cfg.custom_fields.append(String(f))
	for f in file.get_value("manifest", "list_fields", []):
		cfg.list_fields.append(String(f))
	if file.has_section("defaults"):
		for key in file.get_section_keys("defaults"):
			cfg.defaults[key] = file.get_value("defaults", key)
	return cfg


# =========================================================================
# READING AN EXISTING HEADER
# =========================================================================

## Pull field values out of whatever header a file already has.
##
## Permissive on purpose: it reads the same forms the browser's parser does,
## so a file that parses today keeps its values through standardisation. A
## header malformed enough that the parser ignores it may lose a field here
## too, which is the honest outcome rather than a guess.
static func read_header(text: String) -> Dictionary:
	var lines := text.split("\n")
	var head_count: int = mini(lines.size(), 40)
	var has_manifest := false
	for i in head_count:
		if lines[i].contains("MANIFEST"):
			has_manifest = true
			break
	if not has_manifest:
		return {}

	var values := {}
	var quoted := RegEx.create_from_string("\\b([a-z_][a-z0-9_]*)[\"']?\\s*:\\s*([\"'])([^\"']*)\\2")
	var listed := RegEx.create_from_string("\\b([a-z_][a-z0-9_]*)[\"']?\\s*:\\s*\\[([^\\]]*)\\]")
	var commented := RegEx.create_from_string("^[\\s/#*<!-]*([a-z_][a-z0-9_]*):\\s*(.+?)\\s*$")

	for i in head_count:
		var raw := lines[i]
		var m := quoted.search(raw)
		if m:
			values[m.get_string(1)] = m.get_string(3).strip_edges()
			continue
		m = listed.search(raw)
		if m:
			values[m.get_string(1)] = _split_list(m.get_string(2))
			continue
		m = commented.search(raw)
		if m:
			var key := m.get_string(1)
			var val := m.get_string(2).trim_suffix("-->").strip_edges()
			if val.begins_with("["):
				values[key] = _split_list(val.trim_prefix("[").trim_suffix("]"))
			else:
				values[key] = val.rstrip("'\",").strip_edges()
	return values


## A file is a test when its own name says so, or it sits in a tests folder.
## Deliberately narrow: a path merely CONTAINING "test" (a folder called
## _mtest, a module called latest_run) is not a test file, and the loose
## check that matched those mislabelled everything under such a folder.
static func _kind_for(rel: String) -> String:
	var base := rel.get_file().get_basename().to_lower()
	if base.begins_with("test_") or base.ends_with("_test") or base.ends_with(".test"):
		return "test"
	for part in rel.to_lower().split("/"):
		if part == "test" or part == "tests":
			return "test"
	return "module"


## Normalise one feature into a searchable tag.
##
## The features field is the filter bar's vocabulary, so it has to hold TERMS
## rather than prose. A tag is lowercase, hyphenated, and free of the
## punctuation that would break either the per-line parser or a search box:
## "Inject Empty Manifest" and "inject_empty_manifest" both become
## "inject-empty-manifest", so the same idea typed two ways files together
## instead of splitting the index.
static func normalise_tag(raw: String) -> String:
	var t := raw.strip_edges().to_lower()
	var cleaned := ""
	for i in t.length():
		var c := t[i]
		if (c >= "a" and c <= "z") or (c >= "0" and c <= "9"):
			cleaned += c
		elif c == " " or c == "_" or c == "-" or c == "." or c == "/":
			cleaned += "-"
		# every other character is dropped: it cannot survive a search box
	while cleaned.contains("--"):
		cleaned = cleaned.replace("--", "-")
	return cleaned.trim_prefix("-").trim_suffix("-")


## Normalise a whole feature list, dropping empties and duplicates while
## keeping the order somebody wrote them in.
static func normalise_tags(items: Array) -> Array[String]:
	var out: Array[String] = []
	for item in items:
		var tag := normalise_tag(str(item))
		if tag != "" and not out.has(tag):
			out.append(tag)
	return out


static func _split_list(s: String) -> Array[String]:
	var out: Array[String] = []
	for part in s.split(","):
		var t := String(part).replace("'", "").replace("\"", "")
		t = t.replace("[", "").replace("]", "").strip_edges()
		if t != "":
			out.append(t)
	return out


## How many lines the existing header occupies, to drop from the top.
static func _header_span(lines: PackedStringArray) -> int:
	var head_count: int = mini(lines.size(), 40)
	var seen := false
	for i in head_count:
		if lines[i].contains("MANIFEST"):
			seen = true
			break
	if not seen:
		return 0

	var divider := RegEx.create_from_string("^[\\s/#*<!-]*={6,}[\\s-]*(?:-->)?\\s*$")
	var last := -1
	var found_manifest := false
	for i in head_count:
		if lines[i].contains("MANIFEST"):
			found_manifest = true
		if found_manifest and divider.search(lines[i]):
			last = i
	if last < 0:
		return 0
	# Consume one trailing blank so repeated runs do not stack blank lines.
	if last + 1 < lines.size() and lines[last + 1].strip_edges() == "":
		last += 1
	return last + 1


# =========================================================================
# WRITING A HEADER
# =========================================================================

static func _divider(leader: String) -> String:
	var lead := leader + " "
	return lead + "=".repeat(maxi(1, DIVIDER_WIDTH - lead.length()))


static func _render_value(name: String, value: Variant, cfg: Config) -> String:
	if cfg.is_list_field(name):
		var items: Array = value if value is Array else _split_list(str(value))
		# features is the filter bar's vocabulary, so it is normalised into
		# searchable tags. Dependency lists are paths and are left verbatim,
		# because hyphenating a path would break the join to the file it names.
		if name == "features":
			items = normalise_tags(items)
		return "[" + ", ".join(PackedStringArray(items)) + "]"
	return "" if value == null else str(value)


## Wrap a long value across continuation lines.
##
## The parser reads one field per line, so only the first line carries the
## value. Continuations are indented so they read as prose and the parser
## ignores them, which is the shape the existing headers already use.
static func _wrap(leader: String, key: String, value: String) -> Array[String]:
	var lead := leader + "  "
	var first := lead + key + ": "
	var cont := lead + "         "
	var words := value.split(" ", false)
	if words.is_empty():
		return [first.rstrip(" ")] as Array[String]

	var out: Array[String] = []
	var line := first
	for w in words:
		if line.length() + w.length() + 1 > DIVIDER_WIDTH and line != first and line != cont:
			out.append(line.rstrip(" "))
			line = cont
		if line.ends_with(" "):
			line += w
		else:
			line += " " + w
	out.append(line.rstrip(" "))
	return out


## Build the header text for a file in the comment syntax its extension uses.
static func render_header(ext: String, values: Dictionary, cfg: Config) -> String:
	var leader := String(COMMENT_LEADER.get(ext, "#"))
	var out: Array[String] = []
	out.append(_divider(leader))
	out.append(leader + "  MANIFEST")
	out.append(_divider(leader))
	for key in cfg.field_order():
		var rendered := _render_value(key, values.get(key), cfg)
		if key == "description" and rendered.length() > 45:
			out.append_array(_wrap(leader, key, rendered))
		else:
			out.append(leader + "  " + key + ": " + rendered)
	out.append(_divider(leader))
	return "\n".join(PackedStringArray(out)) + "\n"


# =========================================================================
# THE HEADER MARKER
# =========================================================================
# A person types three lines and the cleanup pass turns them into a real
# section divider. The marker is deliberately crude so unfinished output is
# obvious if the pass never runs:
#
#     ======
#     HEADER
#     ======
#
# becomes a full-width divider, the title in capitals, and a closing divider,
# in whatever comment syntax the file uses.

## Expand bare HEADER markers. Returns {"text": String, "count": int}.
static func expand_markers(text: String, ext: String) -> Dictionary:
	var leader := String(COMMENT_LEADER.get(ext, "#"))
	var lines := text.split("\n")
	var marker := RegEx.create_from_string("^[\\s/#*<!-]*={3,}\\s*$")
	var titled := RegEx.create_from_string("^[\\s/#*<!-]*([A-Za-z][^\\n]*?)\\s*$")

	# Skip the manifest block entirely. Its own dividers and field lines look
	# exactly like a marker, so expanding inside it would rewrite a field name
	# as a section title and destroy the value. That is not hypothetical: it
	# ate a features list before this guard existed.
	var protect := _header_span(lines)

	var out: Array[String] = []
	var count := 0
	var i := 0
	while i < lines.size():
		var a := lines[i]
		if i >= protect and i + 2 < lines.size() and marker.search(a) and marker.search(lines[i + 2]):
			var t := titled.search(lines[i + 1])
			if t and not lines[i + 1].contains("MANIFEST"):
				out.append(_divider(leader))
				out.append(leader + " " + t.get_string(1).to_upper())
				out.append(_divider(leader))
				count += 1
				i += 3
				continue
		out.append(a)
		i += 1
	return {"text": "\n".join(PackedStringArray(out)), "count": count}


# =========================================================================
# THE PASS
# =========================================================================

static func _today() -> String:
	var d := Time.get_datetime_dict_from_system()
	return "%04d-%02d-%02d" % [d.year, d.month, d.day]


static func _defaults_for(res_path: String, cfg: Config, existing: Dictionary) -> Dictionary:
	var rel := res_path.trim_prefix("res://")
	var base := res_path.get_file()
	var values := {
		"script_name": base,
		"script_path": rel,
		"module_name": base.get_basename(),
		"version": "0.1.0",
		"description": "",
		"kind": _kind_for(rel),
		"spec": "none",
		"internal_dependencies": [] as Array[String],
		"external_dependencies": [] as Array[String],
		"features": [] as Array[String],
		"api_version": cfg.api_version,
		"last_updated": _today(),
	}
	for key in cfg.defaults:
		values[key] = cfg.defaults[key]
	for key in cfg.custom_fields:
		if not values.has(key):
			values[key] = ([] as Array[String]) if cfg.is_list_field(key) else ""
	# An existing value always wins: this standardises a header, it does not
	# overwrite what somebody wrote.
	for key in existing:
		values[key] = existing[key]
	return values


## Process one file. `check_only` reports what would change without writing.
##
## Returns {"path", "changed", "injected", "markers", "error"}.
static func process_file(res_path: String, cfg: Config, check_only: bool = false) -> Dictionary:
	var result := {
		"path": res_path,
		"changed": false,
		"injected": false,
		"markers": 0,
		"error": "",
	}
	var ext := res_path.get_extension().to_lower()
	if not COMMENT_LEADER.has(ext):
		return result

	var f := FileAccess.open(res_path, FileAccess.READ)
	if f == null:
		result.error = "cannot read: %s" % error_string(FileAccess.get_open_error())
		return result
	var original := f.get_as_text()
	f.close()

	var expanded := expand_markers(original, ext)
	var text := String(expanded.text)
	result.markers = int(expanded.count)

	var existing := read_header(text)
	result.injected = existing.is_empty()

	var lines := text.split("\n")
	var drop := _header_span(lines)
	var body_lines: Array[String] = []
	for i in range(drop, lines.size()):
		body_lines.append(lines[i])
	var body := "\n".join(PackedStringArray(body_lines))
	while body.begins_with("\n"):
		body = body.substr(1)

	var values := _defaults_for(res_path, cfg, existing)
	# Path and name are identity: always the real ones, never a stale copy.
	values["script_path"] = res_path.trim_prefix("res://")
	values["script_name"] = res_path.get_file()

	var next := render_header(ext, values, cfg) + "\n" + body
	result.changed = next != original
	if result.changed and not check_only:
		var w := FileAccess.open(res_path, FileAccess.WRITE)
		if w == null:
			result.error = "cannot write: %s" % error_string(FileAccess.get_open_error())
			result.changed = false
			return result
		w.store_string(next)
		w.close()
	return result


## Walk a directory for files this tool understands.
static func collect(root: String, out: Array[String] = []) -> Array[String]:
	var skip := ["addons", ".godot", ".git", "target", "node_modules", "bin"]
	var dir := DirAccess.open(root)
	if dir == null:
		return out
	dir.list_dir_begin()
	var name := dir.get_next()
	while name != "":
		if name.begins_with("."):
			name = dir.get_next()
			continue
		var full := root.path_join(name)
		if dir.current_is_dir():
			if not skip.has(name):
				collect(full, out)
		elif COMMENT_LEADER.has(name.get_extension().to_lower()):
			out.append(full)
		name = dir.get_next()
	dir.list_dir_end()
	return out
