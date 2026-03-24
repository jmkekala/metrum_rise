#[derive(Clone)]
pub struct DataGrid<T: Clone> {
    pub width: usize,
    pub height: usize,
    pub data: Vec<T>,
}

impl<T: Clone> DataGrid<T> {
    pub fn new(width: usize, height: usize, default_val: T) -> Self {
        Self {
            width,
            height,
            data: vec![default_val; width * height],
        }
    }

    pub fn get(&self, x: usize, y: usize) -> Option<&T> {
        if x < self.width && y < self.height {
            Some(&self.data[y * self.width + x])
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, x: usize, y: usize) -> Option<&mut T> {
        if x < self.width && y < self.height {
            Some(&mut self.data[y * self.width + x])
        } else {
            None
        }
    }

    pub fn set(&mut self, x: usize, y: usize, val: T) {
        if x < self.width && y < self.height {
            self.data[y * self.width + x] = val;
        }
    }

    pub fn in_bounds(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height
    }
}

impl DataGrid<f32> {
    /// Samples a value from the grid using bilinear interpolation.
    /// `x` and `y` are in grid coordinates [0, width) and [0, height).
    pub fn sample_bilinear(&self, x: f32, y: f32) -> f32 {
        if self.width < 2 || self.height < 2 {
            return *self.get(x.round() as usize, y.round() as usize).unwrap_or(&0.0);
        }

        let x0 = (x as usize).min(self.width - 2);
        let y0 = (y as usize).min(self.height - 2);
        let x1 = x0 + 1;
        let y1 = y0 + 1;

        let tx = (x - x0 as f32).clamp(0.0, 1.0);
        let ty = (y - y0 as f32).clamp(0.0, 1.0);

        let v00 = *self.get(x0, y0).unwrap_or(&0.0);
        let v10 = *self.get(x1, y0).unwrap_or(&0.0);
        let v01 = *self.get(x0, y1).unwrap_or(&0.0);
        let v11 = *self.get(x1, y1).unwrap_or(&0.0);

        let v0 = v00 * (1.0 - tx) + v10 * tx;
        let v1 = v01 * (1.0 - tx) + v11 * tx;

        v0 * (1.0 - ty) + v1 * ty
    }
}
