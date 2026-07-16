//! Surface Debug road-tool Godot API methods.

use super::super::super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns compiled road-surface debug line data for editor visualization.
    ///
    /// Uses `try_lock` because this is only a debug/editor helper and should never stall the
    /// Godot main thread while the simulation mutex is busy.
    #[func]
    pub fn get_road_surface_debug_data(&self) -> Variant {
        match self.try_lock_core() {
            Some(mut core) => core.get_road_surface_debug_data_internal().to_variant(),
            None => Variant::nil(),
        }
    }

    /// Returns JSON describing final road-surface triangles under one world-space point.
    ///
    /// Uses `try_lock` because this is a debug/editor helper called from active tools.
    #[func]
    pub fn get_road_surface_probe_debug(&self, world_pos: Vector3) -> GString {
        match self.try_lock_core() {
            Some(mut core) => core.get_road_surface_probe_debug_internal(world_pos),
            None => GString::new(),
        }
    }

    /// Returns terrain render patches that must keep full mesh resolution where compiled road ownership is visible.
    #[func]
    pub fn get_road_locked_terrain_patches(&self) -> PackedInt32Array {
        let core = self.lock_core();
        core.get_road_locked_terrain_patches_internal()
    }
}
