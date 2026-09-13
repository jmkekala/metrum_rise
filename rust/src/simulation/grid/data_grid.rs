// SPDX-License-Identifier: GPL-2.0-only

//! Generic 2D data grid primitive.

/// A flat memory-efficient 2D grid storage.
///
/// Uses row-major indexing: `index = y * width + x`.
#[derive(Clone)]
pub struct DataGrid<T: Clone> {
    /// Grid width (number of columns).
    pub width: usize,
    /// Grid height (number of rows).
    pub height: usize,
    /// Cell values in row-major order.
    pub data: Vec<T>,
}

impl<T: Clone> DataGrid<T> {
    /// Creates a new grid of the given dimensions, pre-filled with `default_val`.
    pub fn new(width: usize, height: usize, default_val: T) -> Self {
        Self {
            width,
            height,
            data: vec![default_val; width * height],
        }
    }

    /// Gets a reference to the value at `(x, y)`, or `None` if out of bounds.
    pub fn get(&self, x: usize, y: usize) -> Option<&T> {
        if x < self.width && y < self.height {
            Some(&self.data[y * self.width + x])
        } else {
            None
        }
    }

    /// Gets a mutable reference to the value at `(x, y)`, or `None` if out of bounds.
    pub fn get_mut(&mut self, x: usize, y: usize) -> Option<&mut T> {
        if x < self.width && y < self.height {
            Some(&mut self.data[y * self.width + x])
        } else {
            None
        }
    }

    /// Sets the value at `(x, y)`. Silently ignores out-of-bounds coordinates.
    pub fn set(&mut self, x: usize, y: usize, val: T) {
        if x < self.width && y < self.height {
            self.data[y * self.width + x] = val;
        }
    }

    /// Returns `true` if `(x, y)` is within the grid dimensions.
    pub fn in_bounds(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height
    }
}

impl DataGrid<f32> {
    /// Samples a value from the grid using bilinear interpolation.
    /// Coordinates use cell indices and clamp to the grid edges. A single row or column
    /// interpolates along its non-collapsed axis; empty grids return zero.
    pub fn sample_bilinear(&self, x: f32, y: f32) -> f32 {
        if self.width < 2 || self.height < 2 {
            if self.data.len() < 2 {
                return self.data.first().copied().unwrap_or(0.0);
            }
            // A single row and a single column both store their remaining axis contiguously.
            let coordinate = if self.width == 1 { y } else { x };
            let lower = (coordinate as usize).min(self.data.len() - 2);
            let fraction = (coordinate - lower as f32).clamp(0.0, 1.0);
            return self.data[lower] * (1.0 - fraction) + self.data[lower + 1] * fraction;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_get_set() {
        let mut grid = DataGrid::new(10, 10, 0);
        assert_eq!(*grid.get(5, 5).unwrap(), 0);

        grid.set(5, 5, 42);
        assert_eq!(*grid.get(5, 5).unwrap(), 42);

        // Out of bounds
        assert!(grid.get(10, 10).is_none());
        assert!(grid.get(5, 10).is_none());
        assert!(grid.get(10, 5).is_none());

        // set should silenty ignore if out of bounds (current behavior)
        grid.set(10, 10, 100);
        assert!(grid.get(10, 10).is_none());
    }

    #[test]
    fn test_bilinear_interpolation() {
        let mut grid = DataGrid::new(2, 2, 0.0);
        grid.set(0, 0, 10.0);
        grid.set(1, 0, 20.0);
        grid.set(0, 1, 30.0);
        grid.set(1, 1, 40.0);

        // Exact corners
        assert_eq!(grid.sample_bilinear(0.0, 0.0), 10.0);
        assert_eq!(grid.sample_bilinear(1.0, 0.0), 20.0);
        assert_eq!(grid.sample_bilinear(0.0, 1.0), 30.0);
        assert_eq!(grid.sample_bilinear(1.0, 1.0), 40.0);

        // Middle
        assert_eq!(grid.sample_bilinear(0.5, 0.5), 25.0); // (10+20+30+40)/4

        // Edges
        assert_eq!(grid.sample_bilinear(0.5, 0.0), 15.0); // (10+20)/2
        assert_eq!(grid.sample_bilinear(1.0, 0.5), 30.0); // (20+40)/2
        assert_eq!(grid.sample_bilinear(0.5, 1.0), 35.0); // (30+40)/2
        assert_eq!(grid.sample_bilinear(0.0, 0.5), 20.0); // (10+30)/2
    }

    #[test]
    fn test_bilinear_clamping() {
        let mut grid = DataGrid::new(2, 2, 0.0);
        grid.set(1, 1, 100.0);

        // Clamping should return the edge value
        assert_eq!(grid.sample_bilinear(2.0, 2.0), 100.0);
        assert_eq!(grid.sample_bilinear(-1.0, -1.0), 0.0);
    }

    #[test]
    fn bilinear_sampling_interpolates_single_axes_and_clamps_their_edges() {
        for (width, height) in [(1, 3), (3, 1)] {
            let mut grid = DataGrid::new(width, height, 0.0);
            grid.data.copy_from_slice(&[10.0, 30.0, 90.0]);
            for (coordinate, expected) in [(-10.0, 10.0), (0.5, 20.0), (1.5, 60.0), (10.0, 90.0)] {
                for collapsed_coordinate in [-10.0, 0.0, 10.0] {
                    let (x, y) = if width == 1 {
                        (collapsed_coordinate, coordinate)
                    } else {
                        (coordinate, collapsed_coordinate)
                    };
                    assert_eq!(
                        grid.sample_bilinear(x, y),
                        expected,
                        "{width}x{height} at ({x},{y})"
                    );
                }
            }
        }
        let point = DataGrid::new(1, 1, 42.0);
        assert_eq!(point.sample_bilinear(-10.0, 10.0), 42.0);
        for (width, height) in [(0, 0), (0, 3), (3, 0)] {
            assert_eq!(
                DataGrid::new(width, height, 0.0).sample_bilinear(0.5, 0.5),
                0.0
            );
        }
    }

    #[test]
    #[ignore = "manual matched release timing of normal and single-axis grid sampling"]
    fn benchmark_bilinear_grid_sampling() {
        use std::{hint::black_box, time::Instant};
        let queries: Vec<_> = (0..4_096)
            .map(|i| ((i % 521) as f32 - 4.5, (i * 37 % 521) as f32 - 4.5))
            .collect();
        for (width, height) in [(1, 1), (1, 512), (512, 1), (512, 512)] {
            let mut grid = DataGrid::new(width, height, 0.0);
            for (i, value) in grid.data.iter_mut().enumerate() {
                *value = (i % 101) as f32;
            }
            let mut samples = [0.0; 11];
            for sample in &mut samples {
                let start = Instant::now();
                for _ in 0..256 {
                    for &(x, y) in &queries {
                        black_box(grid.sample_bilinear(black_box(x), black_box(y)));
                    }
                }
                *sample = start.elapsed().as_secs_f64() * 1_000.0;
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "bilinear_grid_sampling width={width} height={height} samples=1048576 median_ms={:.6}",
                samples[5]
            );
        }
    }
}
