extends SceneTree
const Idx := preload("res://addons/FILE_browser/index.gd")
func _init() -> void:
	for f in DirAccess.get_files_at("res://addons/FILE_browser/pages"):
		DirAccess.remove_absolute("res://addons/FILE_browser/pages/".path_join(f))
	var docs := Idx.build_roots(["C:/Users/David/Documents/metrum_rise/docs",
		"C:/Users/David/Documents/metrum_rise/godot/addons/FILE_browser"])
	var res := Idx.generate_pages(docs, "res://addons/FILE_browser/pages")
	var stray := 0
	for t in res.written:
		var s := GDScript.new()
		s.source_code = FileAccess.get_file_as_string(t)
		if s.reload() != OK:
			stray += 1
	print("written: ", res.written.size(), " error: ", res.error, " unparsable: ", stray)
	quit(0)
