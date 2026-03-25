use godot::prelude::*;
use super::data::RegionGraph;

impl RegionGraph {
    pub const CHUNK_SIZE: f32 = 512.0;
    pub const NODE_CHUNK_SIZE: f32 = 16.0;

    pub fn get_chunk_coords(pos: godot::prelude::Vector3) -> (i32, i32) {
        ((pos.x / Self::CHUNK_SIZE).floor() as i32, (pos.z / Self::CHUNK_SIZE).floor() as i32)
    }

    pub fn get_node_chunk_coords(pos: godot::prelude::Vector3) -> (i32, i32) {
        ((pos.x / Self::NODE_CHUNK_SIZE).floor() as i32, (pos.z / Self::NODE_CHUNK_SIZE).floor() as i32)
    }

    pub fn get_node_chunk(&self, node_id: u32) -> (i32, i32) {
        Self::get_chunk_coords(self.nodes[node_id as usize].pos)
    }

    pub fn add_to_spatial_index(&mut self, edge_idx: usize) {
        let edge = &self.edges[edge_idx];
        if edge.deleted { return; }
        
        // Find all chunks touched by this edge's AABB
        let mut min_x = f32::MAX; let mut max_x = f32::MIN;
        let mut min_z = f32::MAX; let mut max_z = f32::MIN;
        
        for p in &edge.physical_geometry {
            min_x = min_x.min(p.x); max_x = max_x.max(p.x);
            min_z = min_z.min(p.z); max_z = max_z.max(p.z);
        }
        
        let min_c = Self::get_chunk_coords(godot::prelude::Vector3::new(min_x, 0.0, min_z));
        let max_c = Self::get_chunk_coords(godot::prelude::Vector3::new(max_x, 0.0, max_z));
        
        for cx in min_c.0..=max_c.0 {
            for cz in min_c.1..=max_c.1 {
                let chunk = self.spatial_edge_grid.entry((cx, cz)).or_default();
                if !chunk.contains(&edge_idx) {
                    chunk.push(edge_idx);
                }
            }
        }
    }

    pub fn remove_from_spatial_index(&mut self, edge_idx: usize) {
        let chunks = self.get_edge_chunks(edge_idx);
        for coords in chunks {
            if let Some(chunk) = self.spatial_edge_grid.get_mut(&coords) {
                chunk.retain(|&idx| idx != edge_idx);
            }
        }
    }

    pub fn add_node_to_spatial_index(&mut self, node_id: u32) {
        let pos = self.nodes[node_id as usize].pos;
        let chunk_coords = Self::get_node_chunk_coords(pos);
        let chunk = self.spatial_node_grid.entry(chunk_coords).or_default();
        if !chunk.contains(&node_id) {
            chunk.push(node_id);
        }
    }

    pub fn remove_node_from_spatial_index(&mut self, node_id: u32, pos: Vector3) {
        let chunk_coords = Self::get_node_chunk_coords(pos);
        if let Some(chunk) = self.spatial_node_grid.get_mut(&chunk_coords) {
            chunk.retain(|&id| id != node_id);
        }
    }

    pub fn get_edges_near_point(&self, pos: godot::prelude::Vector3, radius: f32) -> Vec<usize> {
        let min = godot::prelude::Vector3::new(pos.x - radius, 0.0, pos.z - radius);
        let max = godot::prelude::Vector3::new(pos.x + radius, 0.0, pos.z + radius);
        self.get_edges_near_aabb(min, max)
    }

    pub fn get_edges_near_aabb(&self, min: godot::prelude::Vector3, max: godot::prelude::Vector3) -> Vec<usize> {
        let mut result = Vec::new();
        let min_c = Self::get_chunk_coords(min);
        let max_c = Self::get_chunk_coords(max);
        
        for cx in min_c.0..=max_c.0 {
            for cz in min_c.1..=max_c.1 {
                if let Some(chunk) = self.spatial_edge_grid.get(&(cx, cz)) {
                    for &idx in chunk {
                        if !result.contains(&idx) {
                            result.push(idx);
                        }
                    }
                }
            }
        }
        result
    }

    /// Returns the coordinates of all chunks (512m) that an edge's AABB overlaps.
    pub fn get_edge_chunks(&self, edge_idx: usize) -> Vec<(i32, i32)> {
        let mut result = Vec::new();
        if edge_idx >= self.edges.len() { return result; }
        let edge = &self.edges[edge_idx];
        if edge.deleted { return result; }
        
        let mut min_x = f32::MAX; let mut max_x = f32::MIN;
        let mut min_z = f32::MAX; let mut max_z = f32::MIN;
        for p in &edge.physical_geometry {
            min_x = min_x.min(p.x); max_x = max_x.max(p.x);
            min_z = min_z.min(p.z); max_z = max_z.max(p.z);
        }
        
        let min_c = Self::get_chunk_coords(godot::prelude::Vector3::new(min_x, 0.0, min_z));
        let max_c = Self::get_chunk_coords(godot::prelude::Vector3::new(max_x, 0.0, max_z));
        
        for cx in min_c.0..=max_c.0 {
            for cz in min_c.1..=max_c.1 {
                result.push((cx, cz));
            }
        }
        result
    }
}
