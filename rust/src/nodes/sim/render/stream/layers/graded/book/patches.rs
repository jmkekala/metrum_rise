// SPDX-License-Identifier: GPL-2.0-only

//! The resident patches' Godot objects as the graded book holds them (`RENDER-32-K` part 7,
//! `docs/terrain.md` "Streaming core"): each patch's node, material and culling box as
//! `terrain.gd` lends them (`set_patch_node`), the draw states applied to the patch nodes (a
//! collapsed `PlaneMesh` while the cells draw it, T53 (a)), the readiness latency (how long a
//! patch drew nothing, and the unready patches in the view), and the test and probe readers that
//! read them, as `terrain_graded_tiles.gd`'s `_resident`, `_collapsed`, `_unready`, `_set_plane`,
//! `_set_ready` and its readers did.
//!
//! The patch nodes are reached by their instance ids. Bounds: a registration, a draw state and a
//! release `O(1)`; the view count `O(unready patches)` a frame (none once the residency has
//! caught up); the readers `O(resident patches)` plus their sets' tiles.

use super::super::cells::{DRAW_NONE, DRAW_PLANE};
use super::super::state::PatchKey;
use super::*;
use godot::classes::{DisplayServer, Engine, GeometryInstance3D, Node3D, RenderingServer, Time};

// A visibility range no camera is within (metres): a patch's PlaneMesh while its cells draw it.
// The node keeps it as a 32-bit float; the readers compare what they read back with this value
// as GDScript's `is_equal_approx` does.
const HIDDEN_RANGE_M: f64 = 0.001;
// The cell meshes of a set holding the behind-the-road variant (`CELL_MESHES`).
const CELL_MESHES: usize = 9;

/// A resident patch's Godot objects: its node's instance id, its material and its culling box.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct PatchNode {
    node: i64,
    material: Rid,
    aabb: Aabb,
}

/// The patches' nodes in the order they were lent (a Godot dictionary's order: `resident_keys`
/// and the readers walk it), the patch nodes whose `PlaneMesh` the book collapsed, the patches
/// drawing nothing since when (engine milliseconds, process frame), and the view's frustum.
#[derive(Default)]
pub(super) struct PatchNodes {
    nodes: OrderedMap<PatchKey, PatchNode>,
    collapsed: OrderedMap<PatchKey, i64>,
    unready: OrderedMap<PatchKey, (i64, i64)>,
    planes: Vec<Plane>,
}

fn geometry(id: i64) -> Option<Gd<GeometryInstance3D>> {
    if id == 0 {
        return None;
    }
    Gd::try_from_instance_id(InstanceId::from_i64(id)).ok()
}

// GDScript's `is_equal_approx` of a float.
fn equal_approx(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() < (0.00001 * a.abs()).max(0.00001)
}

// `Transform3D * AABB`, as Godot's `xform` computes it in real_t.
fn transformed(transform: Transform3D, aabb: Aabb) -> Aabb {
    let min = [aabb.position.x, aabb.position.y, aabb.position.z];
    let end = aabb.position + aabb.size;
    let max = [end.x, end.y, end.z];
    let origin = [transform.origin.x, transform.origin.y, transform.origin.z];
    let rows = transform.basis.rows.map(|row| [row.x, row.y, row.z]);
    let (mut low, mut high) = (origin, origin);
    for i in 0..3 {
        for j in 0..3 {
            let e = rows[i][j] * min[j];
            let f = rows[i][j] * max[j];
            if e < f {
                low[i] += e;
                high[i] += f;
            } else {
                low[i] += f;
                high[i] += e;
            }
        }
    }
    let position = Vector3::new(low[0], low[1], low[2]);
    Aabb::new(position, Vector3::new(high[0], high[1], high[2]) - position)
}

impl GradedTileBook {
    // A changed resident patch (the frame's news, `_patch_changed`): its counted tiles follow its
    // residency, its list is asked again if its generation moved, its cells follow its LOD step;
    // a drawing set whose patch now has another node hides until the book says it draws again.
    pub(super) fn follow_news(&mut self, key: Vector2i) {
        let present = self.is_resident(key);
        let node = self
            .patch_nodes
            .nodes
            .get(&(key.x, key.y))
            .map(|patch| patch.node);
        let node_moved = present && node.is_some_and(|node| self.cells_node_moved(key, node));
        let rect = self.patch_rect(key);
        self.patch_changed(key, present, rect, node_moved);
    }

    // Patch `key` is ready (draws its set or its PlaneMesh) or not (draws nothing): the time and
    // frames from its residency to ready are the readiness latency.
    fn set_ready(&mut self, key: PatchKey, ready: bool) {
        let unready = &mut self.patch_nodes.unready;
        if !ready {
            if !unready.contains_key(&key) {
                let now = (
                    Time::singleton().get_ticks_msec() as i64,
                    Engine::singleton().get_process_frames() as i64,
                );
                unready.insert(key, now);
            }
            return;
        }
        let Some((ms, frame)) = unready.remove(&key) else {
            return;
        };
        let counters = &mut self.counters;
        counters.ready_ms_max = counters
            .ready_ms_max
            .max(Time::singleton().get_ticks_msec() as i64 - ms);
        counters.ready_frames_max = counters
            .ready_frames_max
            .max(Engine::singleton().get_process_frames() as i64 - frame);
    }

    // Patch `key`'s PlaneMesh draws (`drawn`) or not. Collapsed, it is not drawn at all: its
    // visibility range ends before any camera, so its vertices do not run (TERRAIN-07-D round 5).
    // It stays visible, since grass and the water reflection read the patch node's visibility.
    // Drawing again restores the node it collapsed, whatever node the patch holds now:
    // `terrain.gd` pools its patch nodes, and a pooled node left collapsed draws nothing for the
    // next patch it takes (`TERRAIN-07-E` round 6). `node` 0: none.
    fn set_plane(&mut self, key: PatchKey, node: i64, drawn: bool) {
        let collapsed = &mut self.patch_nodes.collapsed;
        let mut held = collapsed.get(&key).copied();
        if let Some(previous) = held.filter(|&previous| drawn || previous != node) {
            collapsed.remove(&key);
            if let Some(mut previous) = geometry(previous) {
                previous.set_instance_shader_parameter("cell_mode", &0.0f64.to_variant());
                previous.set_visibility_range_end(0.0);
            }
            held = None;
        }
        if drawn || held.is_some() {
            return;
        }
        let Some(mut patch) = geometry(node) else {
            return;
        };
        patch.set_instance_shader_parameter("cell_mode", &1.0f64.to_variant());
        patch.set_visibility_range_end(HIDDEN_RANGE_M as f32);
        collapsed.insert(key, node);
    }

    // The patch nodes as a reader walks them.
    fn node_ids(&self) -> impl Iterator<Item = (Vector2i, i64)> + '_ {
        self.patch_nodes
            .nodes
            .iter()
            .map(|(key, patch)| (Vector2i::new(key.0, key.1), patch.node))
    }

    // The residency margin's check (T53 (a)): the unready patches whose culling box (the one
    // Godot culls the patch by) is not wholly outside a plane of the view's frustum.
    pub(super) fn count_unready_in_view(&mut self) {
        let patches = &self.patch_nodes;
        if patches.planes.is_empty() || patches.unready.len() == 0 {
            return;
        }
        let mut in_view = 0;
        for key in patches.unready.keys() {
            let Some(patch) = patches.nodes.get(key) else {
                continue;
            };
            let Ok(node) = Gd::<Node3D>::try_from_instance_id(InstanceId::from_i64(patch.node))
            else {
                continue;
            };
            if !node.is_inside_tree() {
                continue;
            }
            let bounds = transformed(node.get_global_transform(), patch.aabb);
            let outside = patches.planes.iter().any(|plane| {
                // The box corner farthest along the plane's inward side: outside if even it is
                // over.
                let size = bounds.size;
                let corner = bounds.position
                    + Vector3::new(
                        if plane.normal.x < 0.0 { size.x } else { 0.0 },
                        if plane.normal.y < 0.0 { size.y } else { 0.0 },
                        if plane.normal.z < 0.0 { size.z } else { 0.0 },
                    );
                plane.normal.dot(corner) > plane.d
            });
            in_view += i64::from(!outside);
        }
        let counters = &mut self.counters;
        counters.unready_in_view += in_view;
        counters.unready_in_view_max = counters.unready_in_view_max.max(in_view);
    }
}

#[godot_api(secondary)]
impl GradedTileBook {
    /// Patch `key`'s Godot objects (`terrain.gd`, as its node draws the patch): its node's
    /// instance id (0: none, the patch left or has no payload), its material and culling box.
    /// The cell sets draw with them. Returns whether they changed (a trace records a change).
    /// O(1).
    #[func]
    pub fn lend_patch_node(&mut self, key: Vector2i, node: i64, material: Rid, aabb: Aabb) -> bool {
        let at = (key.x, key.y);
        if node == 0 {
            return self.patch_nodes.nodes.remove(&at).is_some();
        }
        let patch = PatchNode {
            node,
            material,
            aabb,
        };
        if self.patch_nodes.nodes.get(&at) == Some(&patch) {
            return false;
        }
        self.patch_nodes.nodes.insert(at, patch);
        self.set_patch_draw(key, node, material, aabb);
        true
    }

    /// The resident patches lent a node, in the order they were first lent (`terrain.gd`'s
    /// overlay sync).
    #[func]
    pub fn patch_node_keys(&self) -> VarArray {
        self.node_ids().map(|(key, _)| key.to_variant()).collect()
    }

    /// Forgets every resident patch and its node (`terrain.gd` cleared its patches; the core
    /// starts over with the world).
    #[func]
    pub fn forget_patches(&mut self) {
        self.patch_nodes.nodes.clear();
        self.clear_resident();
    }

    /// Applies resident patch `key`'s draw state (effect 40): a trace records it; the patch is
    /// ready unless it draws nothing; its PlaneMesh draws only in `DRAW_PLANE`.
    #[func]
    pub fn apply_draw(&mut self, key: Vector2i, state: i64) {
        if let Some(trace) = self.trace.as_mut() {
            trace_out(trace, format!("draw {} {} {state}", key.x, key.y));
        }
        let at = (key.x, key.y);
        self.set_ready(at, state != i64::from(DRAW_NONE));
        let node = self
            .patch_nodes
            .nodes
            .get(&at)
            .map_or(0, |patch| patch.node);
        self.set_plane(at, node, state == i64::from(DRAW_PLANE));
    }

    /// Returns patch `key` to its PlaneMesh (on `node`, its node now; 0: none) and its cell set
    /// to the pool, as the patch recycles.
    #[func]
    pub fn release_patch_node(&mut self, key: Vector2i, node: i64) {
        self.release_set(key);
        let at = (key.x, key.y);
        self.patch_nodes.unready.remove(&at);
        self.set_plane(at, node, true);
    }

    /// Every collapsed PlaneMesh draws again and no patch is unready (the renderer's `clear`).
    #[func]
    pub fn restore_planes(&mut self) {
        let keys: Vec<PatchKey> = self.patch_nodes.collapsed.keys().copied().collect();
        for key in keys {
            self.set_plane(key, 0, true);
        }
        self.patch_nodes.unready.clear();
    }

    /// The view's frustum (`Camera3D.get_frustum()`, set each frame): an unready patch whose
    /// culling box meets it counts in `unready_in_view` (empty: none is counted).
    #[func]
    pub fn set_view_planes(&mut self, planes: Array<Plane>) {
        self.patch_nodes.planes.clear();
        self.patch_nodes.planes.extend(planes.iter_shared());
    }

    /// Resident patches drawing nothing (T53 (a)), for tests.
    #[func]
    pub fn unready_count(&self) -> i64 {
        self.patch_nodes.unready.len() as i64
    }

    /// Resident patches drawing nothing (T53 (a)), and with them `has_pending_work`'s other
    /// parts: queued work that will change what the graded cells draw without a new input (tile
    /// or list requests pending or in flight in Rust, parked tiles, counts, results deferred or
    /// waiting, an edit batch waiting, a preview tile waiting for its variant, and the cell set
    /// builds of patches a shown tile waits for or whose list names a tile). O(queued builds).
    #[func]
    pub fn render_pending(&self) -> bool {
        self.has_pending_work()
            || self.cells_pending()
            || self.lists_pending()
            || self.results_waiting()
            || self.patch_nodes.unready.len() > 0
            || self.has_preview_waits()
    }

    /// Patches resident or not within `radius` of the ground point `centre` (XZ, to the nearest
    /// point of each patch's rectangle) that do not yet draw what they draw once settled: not
    /// resident, their list not arrived (nor refused), or drawing nothing. In `real_t`, as the
    /// renderer computed it. O(patches within the radius).
    #[func]
    pub fn unready_near(&self, centre: Vector2, radius: f64) -> i64 {
        let shape = &self.state.shape;
        let world_min = Vector2::new(shape.world_min[0], shape.world_min[1]);
        let span = shape.patch_span;
        let reach = Vector2::ONE * radius as f32;
        let first = ((centre - reach - world_min) / span).floor();
        let last = ((centre + reach - world_min) / span).floor();
        let mut count = 0;
        for z in (first.y as i32).max(0)..=(last.y as i32).min(shape.patch_grid[1] - 1) {
            for x in (first.x as i32).max(0)..=(last.x as i32).min(shape.patch_grid[0] - 1) {
                let key = Vector2i::new(x, z);
                let origin = world_min + Vector2::new(x as f32, z as f32) * span;
                let nearest = centre.clamp(origin, origin + Vector2::ONE * span);
                if f64::from(nearest.distance_to(centre)) > radius {
                    continue;
                }
                let unready = self.patch_nodes.unready.contains_key(&(x, z));
                count += i64::from(!self.is_resident(key) || unready || !self.list_known(key));
            }
        }
        count
    }

    /// Resident patches drawing their PlaneMesh where T53 (a) forbids it: their list has not
    /// arrived or names a tile that can show, or a preview tile meets them. Read from the patch
    /// nodes, for tests, which check it in every frame: always 0. O(resident patches' lists +
    /// preview tiles).
    #[func]
    pub fn plane_violations(&self) -> i64 {
        let mut violations = 0;
        for (key, node) in self.node_ids() {
            // The node keeps the range as a 32-bit float (`hole_patches`).
            let Some(node) = geometry(node) else {
                continue;
            };
            if equal_approx(f64::from(node.get_visibility_range_end()), HIDDEN_RANGE_M) {
                continue;
            }
            if self.list_drawable(key) || self.previews_in(self.patch_rect(key)) {
                violations += 1;
            }
        }
        violations
    }

    /// Resident patches drawing nothing: their PlaneMesh collapsed with no drawing set of theirs
    /// under it (a hole showing the sky). For tests and the probe; O(resident patches).
    #[func]
    pub fn hole_patches(&self) -> i64 {
        let mut holes = 0;
        for (key, id) in self.node_ids() {
            let Some(node) = geometry(id) else {
                continue;
            };
            if !equal_approx(f64::from(node.get_visibility_range_end()), HIDDEN_RANGE_M) {
                continue;
            }
            holes += i64::from(!self.cells_drawn_under(key, id));
        }
        holes
    }

    /// Resident patches holding a cell set. O(resident patches).
    #[func]
    pub fn patches_with_cells(&self) -> i64 {
        self.node_ids()
            .filter(|&(key, _)| self.patch_set(key) >= 0)
            .count() as i64
    }

    /// For tests: patch key -> its cell set's objects (`cell_objects`, with `set`, `variant` and
    /// `rect`), for every resident patch holding one.
    #[func]
    pub fn patch_cells_view(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        for (key, _) in self.node_ids() {
            let set = self.patch_set(key);
            if set < 0 {
                continue;
            }
            let mut view = self.cell_objects(set);
            let meshes = view
                .get("multimeshes")
                .map_or(0, |meshes| meshes.to::<Array<Rid>>().len());
            view.set("set", set);
            view.set("variant", meshes == CELL_MESHES);
            view.set("rect", self.patch_rect(key));
            out.set(key, view);
        }
        out
    }

    /// Tiles of drawing sets that draw other than what they show (`TERRAIN-12`): the state's
    /// holes and stitches, then per drawing set of a resident patch each cell mesh's instances
    /// as sent against the book's counts and, where a rendering device keeps its data (headless
    /// keeps none), each instance's custom data in the MultiMesh itself against the book's
    /// record. `{holes, stitches}`; for the probe and tests, every frame of a drag; O(drawing
    /// sets' tiles).
    #[func]
    pub fn tile_draw_violations(&self) -> VarDictionary {
        let state = self.draw_violations();
        let mut holes = i64::from(state.x);
        let rendered = DisplayServer::singleton().get_name() != GString::from("headless");
        let server = RenderingServer::singleton();
        for (key, _) in self.node_ids() {
            let set = self.patch_set(key);
            if set < 0 || !self.set_built(set) {
                continue;
            }
            let cells = self.cell_objects(set);
            if !cells.get("shown").is_some_and(|shown| shown.to::<bool>()) {
                continue;
            }
            let counts = self.set_counts(set);
            let sent = cells
                .get("counts")
                .map(|sent| sent.to::<PackedInt32Array>())
                .unwrap_or_default();
            let has = cells
                .get("has")
                .map(|has| has.to::<PackedByteArray>())
                .unwrap_or_default();
            for index in 0..counts.len().min(sent.len()) {
                let (count, sent) = (counts.as_slice()[index], sent.as_slice()[index]);
                if sent != count || (has.as_slice()[index] == 1) != (count > 0) {
                    holes += 1;
                }
            }
            if !rendered {
                continue;
            }
            let multimeshes = cells
                .get("multimeshes")
                .map(|meshes| meshes.to::<Array<Rid>>())
                .unwrap_or_default();
            let instances = self.set_instances(set);
            for at in instances.as_slice().as_chunks::<6>().0 {
                let custom = Color::from_rgba(at[2], at[3], at[4], at[5]);
                let Some(multimesh) = multimeshes.get(at[0] as usize) else {
                    holes += 1;
                    continue;
                };
                if server.multimesh_instance_get_custom_data(multimesh, at[1] as i32) != custom {
                    holes += 1;
                }
            }
        }
        let mut out = VarDictionary::new();
        out.set("holes", holes);
        out.set("stitches", i64::from(state.y));
        out
    }

    /// For tests and the probe: tile key -> `{code, level, step, token, walls, behind, copies}`
    /// of every installed (or `preview`) tile, `copies` mapping a page id to its copy there,
    /// `Vector3i(slot, first atlas layer, layers)`. O(tiles x copies).
    #[func]
    pub fn entries_view(&self, preview: bool) -> VarDictionary {
        let mut out = VarDictionary::new();
        let tiles = self.entry_tiles(preview);
        for pair in tiles.as_slice().as_chunks::<2>().0 {
            let tile = Vector2i::new(pair[0], pair[1]);
            let fields = self.entry(tile, preview);
            let fields = fields.as_slice();
            let mut copies = VarDictionary::new();
            for copy in fields[6..].as_chunks::<4>().0 {
                copies.set(
                    copy[0],
                    Vector3i::new(copy[1] as i32, copy[2] as i32, copy[3] as i32),
                );
            }
            let mut view = VarDictionary::new();
            view.set("code", fields[0]);
            view.set("level", fields[1]);
            view.set("step", fields[2]);
            view.set("token", fields[3]);
            view.set("walls", self.entry_walls(tile, preview));
            view.set("behind", fields[4] == 1);
            view.set("copies", copies);
            out.set(tile, view);
        }
        out
    }

    /// The tiles the last edit batch installed, as `Vector2i`s (tests and the probe).
    #[func]
    pub fn last_edit_batch_tiles(&self) -> VarArray {
        let flat = self.last_edit_batch();
        flat.as_slice()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|tile| Vector2i::new(tile[0], tile[1]).to_variant())
            .collect()
    }
}
