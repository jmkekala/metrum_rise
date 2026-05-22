//! Canonical generated-contact geometry helpers.

mod edges;
mod overlay;
mod point_location;

pub(super) use edges::{
    generated_contact_edges_from_overlay_intersection, generated_contact_edges_inside_contour,
    generated_contact_points_from_contour_intersections,
    generated_directed_edge_segments_inside_shape_edges,
    generated_shape_boundary_segments_on_source_edge,
};
pub(super) use overlay::{generated_overlay_contour, generated_overlay_shapes_directed_edges};
pub(super) use point_location::{
    generated_contour_boundary_contains_key, generated_contour_contains_key,
};
