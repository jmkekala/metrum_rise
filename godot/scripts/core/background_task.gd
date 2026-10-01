# SPDX-License-Identifier: GPL-2.0-only

## Runs a Rust bridge call on the WorkerThreadPool and resumes on the main thread.
## `work` must touch no engine singletons or nodes; it returns the result Dictionary.
extends RefCounted

static func run(tree: SceneTree, work: Callable) -> Dictionary:
	var box := {}
	var task := WorkerThreadPool.add_task(func(): box.result = work.call())
	while not WorkerThreadPool.is_task_completed(task):
		await tree.process_frame
	WorkerThreadPool.wait_for_task_completion(task)
	return box.get("result", {"error": "The background task returned no result."})
