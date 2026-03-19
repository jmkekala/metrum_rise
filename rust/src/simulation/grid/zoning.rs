use godot::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ZoneType {
    None = 0,
    Residential = 1,
    Commercial = 2,
    Industrial = 3,
    Mixed = 4,
}

#[derive(Clone, Debug)]
pub struct ZoningPolygon {
    pub id: u32,
    pub version: u32,
    pub edge_idx: usize,
    pub zone_type: ZoneType,
    pub vertices: Vec<Vector2>,
    pub depth_amt: f32,
    pub frontage_pts: usize,
}

pub struct ZoningSystem {
    pub polygons: Vec<ZoningPolygon>,
    pub next_id: u32,
}

impl ZoningSystem {
    pub fn new() -> Self {
        Self {
            polygons: Vec::new(),
            next_id: 1,
        }
    }

    pub fn clear(&mut self) {
        self.polygons.clear();
        self.next_id = 1;
    }

    pub fn add_polygon(&mut self, edge_idx: usize, zone_type: ZoneType, vertices: Vec<Vector2>, depth_amt: f32, frontage_pts: usize) {
        self.polygons.push(ZoningPolygon {
            id: self.next_id,
            version: 0,
            edge_idx,
            zone_type,
            vertices,
            depth_amt,
            frontage_pts,
        });
        self.next_id += 1;
    }

    pub fn update_polygon(&mut self, id: u32, vertices: Vec<Vector2>, frontage_pts: usize) {
        if let Some(poly) = self.polygons.iter_mut().find(|p| p.id == id) {
            poly.version += 1;
            poly.vertices = vertices;
            poly.frontage_pts = frontage_pts;
        }
    }

    pub fn remove_polygon(&mut self, id: u32) {
        self.polygons.retain(|p| p.id != id);
    }

    pub fn get_render_data(&self) -> PackedFloat32Array {
        let mut data = Vec::new();
        for p in &self.polygons {
            data.push(p.vertices.len() as f32);
            data.push(p.zone_type as u8 as f32);
            for v in &p.vertices {
                data.push(v.x);
                data.push(v.y);
            }
        }
        PackedFloat32Array::from_iter(data)
    }
}
