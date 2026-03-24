//! Spatial grid subsystems: zoning, environmental diffusion, and the generic grid primitive.
//!
//! All grids use [`data_grid::DataGrid<T>`] as their storage primitive — a flat `Vec<T>`
//! with row-major indexing and `rayon::par_chunks_mut` parallelism for tick updates.
//!
//! Tick order within the grid module: pollution → noise → desirability.
//! Desirability reads from pollution and noise, so it must run last.

pub mod data_grid;
pub mod zoning;
pub mod pollution;
pub mod noise;
pub mod desirability;
