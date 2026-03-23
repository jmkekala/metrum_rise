// Map and Global Simulation Limits
pub const MAP_WIDTH: usize = 1000;  // MAP SIZE (WIDTH)
pub const MAP_HEIGHT: usize = 1000; // MAP SIZE (HEIGHT)
pub const HEIGHT_SCALE: f32 = 20.0; // Vertical exaggeration for terrain height
pub const SNAP_TOLERANCE: f32 = 1.0; // Road node snapping distance
pub const INTERSECTION_TOLERANCE: f32 = 1.0;

// Road Geometry and Rendering Constants
pub const ROAD_H_OFFSET: f32 = 0.01; // Elevation of road mesh above terrain (prevents Z-fighting)
pub const LANE_WIDTH: f32 = 3.5;     // Width of a single lane
pub const SIDEWALK_WIDTH: f32 = 1.5; // Width of a single sidewalk
pub const Z_FIGHT_BIAS: f32 = 0.001; // Fine-tuning for overlay meshes

// Zoning Simulation Parameters
pub const ZONING_DEPTH: usize = 10; // Number of cells deep from the road (e.g., 10 cells * 10m = 100m)
pub const GRID_CELL_SIZE: f32 = 10.0; // The physical size (meters) of a single zoning grid cell
